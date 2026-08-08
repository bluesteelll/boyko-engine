# Logging — the sink, the lifecycle, and the synchronous channel

<!-- CONTRACT
provides: logging/sink-lifecycle
assumes:  substrate/mute-leaf-rule
assumes:  substrate/clock-source
assumes:  seam/free-when-off
assumes:  seam/lifecycle-order
assumes:  seam/vocabulary
assumes:  logging/budgets-and-invariants
assumes:  logging/emission-path
assumes:  logging/ring-and-statics
-->

**Carved from `docs/LOGGING-SYSTEM-PLAN.md` (v4)** — Decisions 9, 9b, 9c, 10, 11, 12, 22, 23, 24, 26 in full; Decision 25's runtime half only; §Data structures' sink blocks; §Public API's lifecycle and control slice; §Algorithms C, D, E; §Multithreading model's consumer half. Diff against the monolith until it is retired.

Everything in this file is downstream of the ring. The producer path — the three gates, the 20 B header, `LogLane`, `CONTROL`, the admission arithmetic — is `logging/emission-path` and `logging/ring-and-statics`; this file starts where the consumer role begins.

---

## Decision 9: `report!` is DELETED. No engine code writes stdout at all *(re-cut by S1)*

v3 specified `report!` as a synchronous stdout macro carrying `VB-P1d` / `VB-P4` / `VB-SV0-S1.5` and the R0 table. **S1 gives the measurement channel to the profiler end to end**, so `report!` is deleted from this plan together with mandatory test 16 and L8b's 20 measurement rows.

**The rule that replaces it, stated once** *(S7 — the seam owns the invariant; this is its restatement at the decision that obeys it)*:

> **stdout is written by exactly one thing in this workspace: `boyko_shaderdsl`'s CLI bins.** Nothing in the engine, the logger or the profiler writes stdout, ever. The measurement channel's durable output is the profiler's **artifact + binary telemetry stream (files)**, rendered by `tools/prof_decode` offline and by the `boyko_ui` overlay in-process.

**Why `report!` could not survive, with the measurement.** v3 justified it by "the lines are a machine API". They are — but the review that examined the seam found **six** consumer files, not one: `crates/boyko_app/tests/vg_occ_split_timing.rs`, `vb_bench_totality_gate.rs`, `vb_bench_query_validation.rs` (which uses the line as a *liveness witness* that the reset and every timestamp write executed), `vg_decidability_floor.rs`, `vb_p1d_cull_shade_bench.rs` and `sv0_deferred_term_bench.rs` (which transcribes printed lines into test source). All six exist; all six were checked. `vg_decidability_floor.rs` is decisive: the profiler's own `band = max(floor, twin)` consumes a **floor produced from that stdout line**, so keeping the line as `report!` would freeze a text stdout contract for measurement permanently *and* hand every machine-parsed line the `OUT_LOCK` steal interleave (G18 concedes it) — the interim design this project has standing instructions never to propose.

**The cost to this plan, stated plainly.** L8b's headline "20 sites → `report!`, text unchanged" drops to **zero rows**; the migration denominator falls from ≤ 98 to **≤ 78**; test 16 and the `report!` half of `print_allowlist.txt` are struck; open question 1 (a schema-versioned TOML form for `report!`) is moot and struck. The cost to the *engine* is larger and belongs to the profiling plan: six consumer files are rewritten in one commit and **every published floor number is invalidated**, because a floor measured on a different instrument bounds nothing about this one.

**What survives.** `OUT_LOCK` is **not** deleted — its remaining callers are enumerated in Decision 9c, and Decision 9c is retained unchanged. `write_oracle_line` is not deleted; it gains the durable fan-out (B9). The byte-frozen `[vk-validation]` contract is untouched (Decision 9b). **This plan moves no stdout contract**, which was true in v3 and is true in v4 for a stronger reason: it no longer writes stdout at all.

**RED for the deletion** *(S1)*: after profiling rung 7, `rg 'VB-P1d |VB-P4 pass=|VB-SV0-S1\.5 ' crates/*/src` must return **zero**. Leaving one `println!("VB-P4 pass=…")` in `runner.rs` is caught twice — by that grep and by L8c's `print_census.rs`.

---

## Decision 9b: The validation messenger is NOT TOUCHED AT ALL — v1's migration stays withdrawn, and v2's "harmless" edit is withdrawn too *(fixes B8 and F12)*

**What.** `crates/boyko_rhi_vulkan/src/debug.rs`'s callback keeps its `eprintln!("[vk-validation] {}", msg.to_string_lossy())` at `:114`, byte for byte, on `stderr()`'s own lock. Nothing about it changes. It is added to `tests/print_allowlist.txt` with the reason "byte-frozen gate-oracle channel; see Decision 9b" — and because that allowlist is checked **in both directions**, a future removal of the site reds the tidy test rather than silently orphaning the entry.

*(Re-verified this session: `crates/boyko_rhi_vulkan/src/debug.rs:114` is `eprintln!("[vk-validation] {}", msg.to_string_lossy());`, with `CStr::from_ptr(data.p_message)` at `:113`.)*

**Why v2's edit is withdrawn** *(F12)*. v2 justified touching the site by "removal of the per-message `to_string_lossy()` allocation". That justification is largely false and the edit is actively harmful:

- `CStr::to_string_lossy()` returns `Cow::Borrowed` for valid UTF-8 — **no allocation on the normal path**. It allocates only for invalid UTF-8, which is not the path any gate runs.
- Writing "the `CStr` bytes directly" **changes the emitted bytes** exactly in the invalid-UTF-8 case (today `U+FFFD`, after: raw bytes) — on a channel this document declares byte-frozen and gate-oracle, pinned byte-exact including the trailing space at `crates/boyko_app/tests/vb_bench_query_validation.rs:118` *(v4's `:116-118` is off by two: `VALIDATION_PREFIX: &str = "[vk-validation] "` is at `:118`, and its own comment two lines above calls it "the gate's entire input"; the parse sites are `:345` and `:348`)*.
- `eprintln!` currently takes `stderr()`'s own lock. Moving the site to `OUT_LOCK` would let the ~90 surviving `eprintln!` sites interleave *inside* a `[vk-validation]` line — a regression **introduced by M24's own fold**.

Trading a non-allocation for a byte change and an interleaving hazard, on the one channel whose value is that it has not moved, is a bad trade. The site stays.

**Why the channel stays synchronous.** Verified this session at `scripts/golden.ps1:201`, `:226`, `:232`: the scan runs over the child's *merged* stdout+stderr file (`cargo … --nocapture > "$valLog" 2>&1` executed through `cmd /c`, because PS 5.1 wraps native stderr into `NativeCommandError` records) and prints `VALIDATION: clean (0 messages).` in green at zero. Today the message is on the wire before `vkQueueSubmit` returns. Behind a 16 KiB lane drained ≤ 8 ms later, three loss modes are all reachable *in exactly the runs the gate exists for*: a storm overflows the lane (a storm is what an error looks like); an error preceding a driver abort loses everything undrained; a rate policy suppresses. Each yields green. **Decision 9's own rule — a gate whose evidence can vanish is worse than no gate — applies here verbatim, and v1 violated it.**

**The E12 conflict is resolved, not finessed.** "No lock, no syscall" is a rule about **frame-hot paths**. A validation callback under an enabled validation layer is not one: validation is off by default, and when on, the run is already an order of magnitude slower. With the site untouched, the conflict does not even arise — the lock in question is `stderr()`'s, which predates this plan.

**The shared-handle clause, which is what makes S7's stderr rule buildable.** Both stderr producers — this messenger and `boyko_log::write_oracle_line` — write through `std::io::stderr()`'s **own handle**, never a raw fd, never `libc::write`. That is what makes them share stderr's inner lock, so **neither can splice a line into the other**, and `golden.ps1:226`'s line-start match on `[vk-validation] ` keeps holding under concurrency. *Ordering* between the two is undefined and stated as such: a log line may land between two `[vk-validation]` lines. **Line integrity, not ordering, is what the gate consumes** — and line integrity is exactly what the shared handle buys. Under an `OUT_LOCK` **steal** two `write_oracle_line` outputs may interleave with each other; `OUT_STEALS > 0` in a golden run remains a defect signal (Decision 9c, E25).

**What is *added*:** `boyko-E2101` (Decision 6's registry row; the emitter is the behaviour-change table in `logging/registry-and-walker`) and the `LOG-CENSUS`, both of which make *absence* loud rather than making presence prettier. Neither writes to the messenger's channel. **Nothing is added that names a `vk-validation` *target***: a census row for a target no record can ever reach is a green-because-it-cannot-fail row wearing the vocabulary invented to prevent them, and it is deleted. The full argument is `logging/goal-and-audiences`' sync-validation confrontation.

---

## Decision 9c: `OUT_LOCK` — bounded acquire, re-entrancy-aware, unwind-safe, and it steals rather than hangs *(fixes F8)*

**`OUT_LOCK` survives S1.** Deleting `report!` deletes one caller, not the lock: the **complete remaining caller list** is `write_oracle_line` (which is itself the console sink, the sync-routed targets of Decision 20, the lane-exhaustion fallback of Decision 5, the pre-`enable` / post-`shutdown()` `Warn`/`Error` fallback of Decision 12, the panic message, and `flush()`'s timeout line) plus `SINK_REQ` writes. Five of those seven are error-of-the-error paths, so the protocol below is if anything **more** load-bearing after S1, and G18 keeps its subject unchanged.

v2 specified an `AtomicBool` spin lock with **no bound, no release-on-unwind and no re-entrancy story**. Three concrete hangs followed, each on the error-of-the-error path — the one place a logger must not fail:

- **E14** ("panic inside a sink — the sink catches, **direct-writes**, continues"): if the panic happened while the sink held `OUT_LOCK`, the direct-write is a non-reentrant self-deadlock.
- **`flush()`'s timeout** writes `boyko-E0105` via `write_oracle_line` (Algorithm D step 5) — a *bounded* wait terminating in an *unbounded* one.
- A `Display` panicking inside a formatting call leaked the lock permanently; the panic hook's flush then hung the process.

Against an invariant the same plan states as "no new hang class", citing `crates/boyko_app/tests/vb_bench_totality_gate.rs:48-49` — verified verbatim this session: *"no host-side frame cap can reach a driver call that never returns, and this repository has no kill-after-timeout pattern to borrow. … a worker A that never terminates is a RED whose message is its own silence."* The protocol is therefore specified, not assumed:

```rust
static OUT_OWNER: AtomicU64 = AtomicU64::new(0);   // 0 = free; else an opaque thread token
static OUT_STEALS: AtomicU32 = AtomicU32::new(0);
static OUT_REENTRANT: AtomicU32 = AtomicU32::new(0);

/// RAII. `Drop` releases on the normal path AND on unwind.
struct OutGuard { mode: OutMode }   // Held | Reentrant | Stolen
```

1. **Format before you lock.** Every caller renders into a caller-owned stack buffer first. No user `Display` and no `core::fmt` runs inside the critical section, so an unwind cannot originate there.
2. **Re-entrancy is detected, not deadlocked.** Acquire is `CAS(0 → my_token)`. On failure, if `OUT_OWNER == my_token` the caller is re-entrant (the E14 case): the guard is `Reentrant`, the bytes are written **prefixed by a newline** so they cannot corrupt the *start* of the outer line, and `OUT_REENTRANT` increments. The census reports it.
3. **Acquire is bounded.** Spin with `spin_loop()` backoff, then `yield_now()`, to a **50 ms** deadline. On expiry the writer **steals**: it writes anyway, increments `OUT_STEALS`, and emits `boyko-W0110` once. An interleaved line is a legible defect; a hung process is not. This is the explicit trade, and it is the only shape compatible with Invariant 6 (`logging/budgets-and-invariants`).
4. **Release is unwind-safe** by construction — `Drop` on `OutGuard`, and the guard is the only way to obtain write access.
5. **The panic hook and `flush()`'s timeout path use the same bounded acquire**, so no bounded wait terminates in an unbounded one.

### The durable fan-out *(fixes B9)*

v3's `write_oracle_line` targeted **stderr unconditionally**, and three separate mechanisms leaned on it for durability: the sync route ("this must be on disk before the next instruction", Decision 20), the lane-exhaustion fallback for `Warn`/`Error` (Decision 5, E6), and E22's crash mitigation. In `shipping` and `shipping-min` there is no console sink, so all three wrote to a stream nothing collects — inert in exactly the configurations they exist for.

**What is true about stderr on this tree, measured**: `grep -rn windows_subsystem crates src` returns **nothing** *(re-run this session: still nothing)*, so today every binary is a console-subsystem binary and stderr is a valid handle. The invalid-handle case is a **future** shipping configuration (`#![windows_subsystem = "windows"]`), not a current fact, and this document does not claim otherwise. The *durability* defect, however, is present today in every profile including `dev`: stderr is neither the log file nor `fsync`ed, so "durable-on-write" was already false.

`write_oracle_line(prefix, body)` therefore writes to **every configured synchronous destination**, under one `OutGuard`:

| Destination | Present when | Cost |
|---|---|---|
| `std::io::stderr()`'s handle | a `ConsoleSink` is configured (`Dev`, `Editor`) | one `write_all`, sharing stderr's inner lock (S7) |
| the **crash sink's** file handle, opened at `enable()` *(was `boot()` — S13, below)* | a `CrashSink` is configured (`Shipping`, `ShippingMin`, and `Dev` on request) | one `write_all` on an append handle; `sync_data()` **only** when `LogConfig.sync_durable` is set |
| the file sink's handle | **never** — that handle is owned by the sink thread and is not reachable synchronously without adding a second consumer | — |

Three consequences, stated rather than implied.

**(i)** "Durable-on-write" now means what a `write_all` means: the bytes have left the process. Reaching the platter additionally needs `sync_data()`, which costs ~0.1-10 ms and is therefore **opt-in** (`sync_durable`, default off) — a per-target sync bit that also `fsync`ed would serialise the frame on the disk rather than on the format.

**(ii)** The **~200 ns in Decision 20 is the uncontended, console-only cost**; with a crash-file destination it is one further `write_all`, and **contended it is bounded only by the 50 ms acquire deadline**, after which the writer steals (E25) — so a sync-routed record can interleave with another synchronous line, which is exactly the property such records exist to avoid. That is the trade, and Decision 20's "cannot claim" now names it (`logging/game-facing-surface`).

**(iii)** In `off`, and equally **on any flag-off run** (S13), there is no synchronous destination, `write_oracle_line` is a no-op, and the mechanisms depending on it are dead — correct in the `off` build because it deletes every call site, and correct on a flag-off run because a player who did not ask for diagnostics gets none. The two cases differ in *why* and the plan says which is which: `off` deletes the site; flag-off leaves the site and gives it nowhere to write.

**Gate G18** (L3-gate), now three-sided: (a) a thread that acquires `OUT_LOCK` and then panics releases it — a second thread's `write_oracle_line` completes within the deadline; (b) a re-entrant `write_oracle_line` from inside a sink panic handler **completes** and increments `OUT_REENTRANT` instead of deadlocking; **(c) fan-out**: with a console sink **absent** and a crash sink configured, a `Warn` from a laneless thread appears in the **crash file**. **Red states**: replace the guard with a bare `store(false)` after the write ⇒ (a) hangs and the test's own deadline reds it; restore the unconditional-stderr form of `write_oracle_line` ⇒ (c)'s crash file is empty ⇒ red. **What G18 cannot claim**: that the output is never interleaved. Under a steal it *is*, deliberately; `OUT_STEALS > 0` in the census is the honest report of that, and a nonzero value in a golden run is itself a defect signal.

**`OUT_LOCK` gets no row in `docs/HOT-PATH-EXCEPTIONS.md`** — Invariant 1 (`logging/budgets-and-invariants`) records why that would red CI: `scripts/check_hotpath_exceptions.py` requires the registry row count per file to match that file's `#[allow(clippy::disallowed_types)]` count, `OUT_LOCK` is an `AtomicU64` which the type ban does not cover, so `sync_out.rs` carries no `#[allow]` and a row for it would be drift ⇒ exit 1. Its justification lives in `sync_out.rs`'s module doc and here.

---

## Decision 10: One dedicated sink thread; adaptive park; **three** consumer disciplines, not two *(re-cut by B8; lifecycle re-cut by S13)*

**What.** Default `SinkMode::Thread`: one thread, **started at `enable()`** *(was `boot()` — see S13 below)*, draining all lanes, staging, sorting by `tsc`, formatting once, fanning out. Park policy is **adaptive**: `park_timeout(0)` — immediate re-drain — while any lane yielded records last pass; `park_timeout(8 ms)` when every lane was empty. Producers **never** unpark (that is a syscall per record); `flush()` and `shutdown()` do.

**`SinkMode` has three variants, because two were not enough** *(B8)*:

| Mode | Consumer | Used by |
|---|---|---|
| `Thread` (default) | the resident sink thread | `Dev`, `Editor`, `Shipping` |
| `Manual` | an explicit `drain()` call, single-caller | hermetic tests, CLI tools, the zero-alloc gate — **and nothing else** |
| **`Scheduled`** *(new)* | **`log_drain_system` in `Last`**, which takes `DRAIN_OWNER` and runs Algorithm C itself, once per frame, on the frame thread | **`ShippingMin`** |

**Why `Scheduled` had to exist.** v3 gave `shipping-min` `SinkMode::Manual` and a crash sink. Nothing then drains: `flush()` returns `NoConsumer` immediately, admission control **drops new records** rather than overrunning oldest, and within seconds the 80 × 16 KiB of lanes hold nothing but boot-time records while everything up to the crash is refused. The profile whose only product is a crash log was structurally guaranteed **not to contain the crash** — and Decision 25 asserted `Manual` for it while this decision said `Manual` exists "for hermetic tests, CLI tools and the zero-alloc gate, and for nothing else", so the two contradicted each other on the page.

**Why draining from the schedule is admissible now, when v3 rejected it.** The rejection below stands on two grounds, and one of them has been removed:

- *"It makes the consumer MPSC when combined with any other driver."* **Answered by `DRAIN_OWNER`** (Decision 24, B5): the consumer role is a CAS-claimed token, so a schedule drain, a manual `drain()` and the crash drain are mutually exclusive by construction rather than by convention.
- *"It ties log liveness to a running schedule, so boot and shutdown diagnostics vanish."* **Not answered — and therefore stated as the profile's cost.** In `ShippingMin`, records emitted before the first frame or after the last are covered only by the synchronous channel (`sink_can_accept()`'s pre-enable / post-shutdown fallback, Decision 12) and by the crash drain. That is a real hole in a real profile, written here rather than discovered by a support agent.

**The other cost, stated in the units it is paid in.** The drain runs **on the frame thread**, bounded per pass by `STAGE_BYTES` (256 KiB of staged records) plus the format cost of what it staged. At `shipping-min`'s `Warn` ceiling the expected volume is a handful of records per second, so the expected per-frame cost is ~0; the **worst** case is one `STAGE_BYTES` pass, which is why the pass is bounded at all. `Thread` mode remains the default everywhere it is affordable.

**The retained window, in RECORDS** *(B8 asks for this explicitly, because bytes are not what a reader wants to know)*: 80 lanes × 16 KiB ÷ ~40 B ≈ **32 800 records** across all lanes, ≈ **410 per lane**. **The per-lane figure is the one that matters and Q1 did not move it** — the total read 13 100 at 32 lanes, and it grew only because there are more lanes, not because any lane got deeper. Under `Scheduled` that ceiling is never approached — the window is "records since the previous frame" — and the crash drain therefore emits the records adjacent to the crash rather than the session's first 32 800. Under `Manual` with no caller (the v3 shape) the same records were guaranteed to be the *oldest* ones.

**Throughput, stated rather than deferred** (M19). At the default geometry, a lane holds `16 KiB / ~40 B` ≈ 400 records; with immediate re-drain under load, per-lane sustained capacity is bounded by the sink's formatting rate, not by the park interval. Design number: **≥ 500 K records·s⁻¹ aggregate** with a `core::fmt` cost of ~1-2 µs per formatted record on one thread. Consequence, stated plainly: **a `trace!` inside a per-entity loop is lossy by construction** — at 15 ns/record a single producer can offer 66 M records·s⁻¹ against a consumer two orders of magnitude slower. Gate **`sink_sustained_rate`** at L3 measures the knee and must show a nonzero drop count above it; the plan is not allowed to ship with the knee unmeasured.

**Alternatives rejected.** *Drain from an ECS system at `Last` **as the default*** — ties log liveness to a running schedule, so boot and shutdown diagnostics vanish; admitted for `ShippingMin` only, with that cost written down (`Scheduled`, above). *Drain from the frame loop* — a syscall in the frame. *Making `ShippingMin` overrun-oldest instead* — see Decision 5 (`logging/ring-and-statics`): it destroys the record that reported the cause in favour of the one that reported the consequence, and the profile does not need it once it has a consumer.

---

## Decision 11: The clock is `boyko_diag::clock` — this crate owns no clock at all *(re-cut by S4)*

**What.** `crates/boyko_log/src/tsc.rs` is **deleted**. Timestamps come from `boyko_diag::clock` (`substrate/clock-source`): `ticks()` (`rdtsc`, QPC fallback), `ticks_per_ns()`, `clock_epoch()`, `calibrate()`, `note_forward_jump()`, `invariant_tsc()`. Records store **raw ticks**; the sink reads the scale and the epoch from the substrate. Code **W0101** (invariant TSC absent) is **struck** — it had no reachable red state on any targeted machine by this plan's own N30, and the single invariant-TSC code is now the profiling plan's `boyko-W9207`. *(Named bare, per the corpus rule in `logging/registry-and-walker`: a struck code has no registry row, so writing its prefixed literal in a scanned document would red check 4 — and this file is now **inside** `docs/diagnostics/**`, which is in that corpus.)*

**Why one owner, and what the sharing actually buys.** Not speed: the boot saving is ~one `cpuid`, not 20 ms. **The benefit is agreement**, and this document says so rather than claiming a speedup. Without one owner a suspend/resume produces a profiler window quarantined as an epoch break and, in the same seconds, log lines whose printed wall times are wrong by the suspend duration **with no marker** — two artifacts that disagree, neither of which says why. *(This is S4's justification; the seam record states it as the decision, `SEAM.md` §S4. It is restated here because it is the reason the sink renders what it renders, and a sink whose epoch rendering has no argument attached is the thing that gets "simplified" away.)*

**`RecordHeader` carries the epoch, and the header does not grow** *(S4 left the choice to L3; here it is)*. v3's header was 20 B packed with `flags: u8` (3 bits used) and `_pad: u8`. v4 spends the pad: **`clock_epoch_lo: u8`**, the low 8 bits of `boyko_diag::clock_epoch()`, read as a register-resident global — no probe, no syscall, so the ≤ 15 ns row is unaffected and the `HEADER_BYTES == 20` const assert stands. Eight bits suffice because **the sink is at most one park interval behind the producer, so at most one epoch boundary can lie between them**: the sink reconstructs the full `u32` by comparing `clock_epoch_lo` against the current `clock_epoch()`. The sink **renders the epoch beside every timestamp**, so a record straddling a discontinuity is legible instead of merely wrong. *(The header's byte layout itself is `logging/emission-path`'s; what is owned here is the consumer-side reconstruction rule and the rendering obligation.)*

**The citation, corrected in v3 and carried.** The tree's note on the QPC-backed `Instant::now()` is at `crates/bench_bevy_vs_boyko/benches/profile_spawn.rs:229-231`: "each **pair** of `now()` calls costs **~20-30 ns**". v2 claimed "~25 ns/call and ~60 ns/pair — *measured*", which is 2× the cited source and attributes a measurement to a prose comment. The tree records ~20-30 ns per *pair*, i.e. ≥ 10 ns per call, and that number is a comment, not a recorded run. The argument survives with room to spare — a ≥ 10 ns clock inside a 15 ns whole-record budget is not a design. Tracy uses `rdtsc` guarded by an invariant-TSC check; AVX2 baseline ⇒ Haswell+ ⇒ invariant TSC since Nehalem.

**Cross-domain correlation, restated with what sharing gained.** A host clock is still not comparable to a Vulkan GPU timestamp without `VK_EXT_calibrated_timestamps` (per Khronos, device timestamps "cannot be compared even across separate submits within the same run"), and GPU correlation remains out of scope. What **is** now exact is **CPU ↔ log-record correlation**: same counter, same scale, same epoch, so a profiler sample and a log record can be placed on one axis without a fitted offset. That is the only cross-domain correlation v1 of either subsystem offers, and it exists *because* the clock is shared.

---

## Decision 12: No `LogHandle`; explicit `boot`/`enable`/`shutdown`; `flush` never waits on a consumer that cannot answer *(fixes M16, M17; lifecycle re-cut by S13)*

**What.**

- `boot(cfg) -> Result<(), LogBootError>`; state lives in process-lifetime statics. **There is no handle**, because v1's handle was `!Send + !Sync` (so it could not be a `Resource`, which requires `Send + Sync`) and its `Drop` would have shut the logger down at the end of `Plugin::build`.
- **`enable(spec) -> Result<(), LogEnableError>`** *(new at S13)*. `boot()` is now a **pure struct-fill** — it validates `LogConfig`, resolves target ids and returns; it **spawns nothing, opens nothing, installs nothing and touches no lane buffer**. Everything that costs a syscall, a thread or a page moves here. See §"Free when not enabled" below for the full move table and why the table is where it is.
- `shutdown()` is explicit, idempotent, called by `App` teardown and by the process-exit path. It is a no-op when `enable()` never ran, because there is nothing to stop.
- `flush() -> FlushResult` reads `SINK_STATE` **first**: `NotBooted` / `Manual` / `Scheduled` / `Exited` ⇒ return `FlushResult::NoConsumer` immediately. Only in `Running` (or `Exiting`) does it bump `FLUSH_SEQ`, unpark, and spin to a 2 s deadline, after which it direct-writes `boyko-E0105` and returns `FlushResult::TimedOut`.
- `shutdown()` sets an exit flag, unparks, then spins on `SINK_EXITED` to a 2 s deadline. **There is no `join`-with-timeout, because `std::thread::JoinHandle::join` does not have one** — v1 asserted a facility std does not provide, in the one place it promised "no new hang class". On timeout the thread is **detached** and `boyko-E0108` is written synchronously.

**Why it matters.** This repo has dozens of `#[should_panic(expected = "boyko-B…")]` tests and a panic hook that flushes. With v1's unconditional 2 s deadline, every such test in an unbooted binary would have paid the full timeout — a self-inflicted 60-second test suite. Under S13 the property strengthens: an un-*enabled* binary is in the same short-circuit state, so the zero-cost path is the one every test and every flag-off player run takes.

### The lifecycle is `boyko_app`'s, and it is stated once *(S5)*

Nobody but `boyko_app` may call `boot`/`enable`/`shutdown`. **The order is the seam's** — it is written once in `SEAM.md` under `seam/lifecycle-order`, because a boot order stated in two plans is exactly the object this corpus was split to prevent. What this file owns is the *behaviour of each verb*, not the order they are called in. The one clause worth repeating because it is this plan's own fix: **`flush_gpu` moves ahead of `flush`**, closing the teardown hole where GPU-side diagnostics were emitted after the logger had stopped accepting them.

### `PRE_FLUSH` — the callback seam, owned here

```rust
/// .bss, claimed by CAS, holding `extern "C" fn()`. Called by flush(), by
/// shutdown(), and by the panic hook at step 1.5 — BEFORE the crash drain.
static PRE_FLUSH: [AtomicPtr<()>; 8] = [const { AtomicPtr::new(null_mut()) }; 8];
pub fn register_pre_flush(f: extern "C" fn()) -> Result<(), PreFlushFull>;
```

A registrant's contract, asserted per registrant and **not** provable in general: **no allocation, no lock, one `write_all`, and it must not touch the `World`**. The profiler's `flush_on_panic` obeys it by moving its telemetry double buffer and file handle out of the `Profiler` `Resource` into a process-static — consistent with S12's extent rule, since both extents are compile-time constants.

**Eight slots is a hard cap; a ninth registration returns `Err` and emits `boyko-E0118`.** *(The seam record illustrated this as `E0110`. That number is taken: `W0110` is `OUT_LOCK`'s steal code, `DIAGNOSTICS` is dense with `index == code_idx`, and registry check 1 asserts numbers strictly increasing — two rows numbered 110 would not compile. `E0118` is the next free slot in the `01xx` band. Deviation recorded in the seam disposition.)*

The array is `.bss` and zero-initialised, so **an un-enabled process has eight null slots and no registrant** — the array costs address space and nothing else. Registration itself moves onto the enable path (S13).

### `sink_can_accept()` — one predicate closes both lifecycle holes

```rust
#[inline] fn sink_can_accept() -> bool;   // one Relaxed load of SINK_STATE
```

When it is **false** (`NotBooted` or `Exited`) **and** the level is `Warn`/`Error`, the record takes the **synchronous channel** instead of being dropped. That closes the pre-`enable()` hole and the symmetric post-`shutdown()` hole with one branch. Cost: one extra load plus a predicted-not-taken branch on the **failed-gate** path of `warn!`/`error!` only — `info!`/`debug!`/`trace!` are untouched, so the ≤ 3 ns row stands and the `log_disabled_warn ≤ 4 ns` row (`logging/budgets-and-invariants`) is what bounds the addition.

**What it does NOT buy on a flag-off run, stated so nobody infers otherwise.** With the runtime flag off there is no configured synchronous destination, so `write_oracle_line` is inert (Decision 9c consequence (iii)) and a severe record emitted before `enable()` reaches **nothing**. That is the correct behaviour for a player who did not ask for diagnostics, and it is *not* the same statement as "the record was dropped by admission control" — it was never offered to a sink, because no sink exists. The census cannot report it either, for the same reason. This is the honest boundary of the predicate and it is written here rather than left to be discovered.

**Deferred diagnostics.** Any condition observed *below* the logger or *before* it is `boyko_diag::raise(DiagFlag)` plus a counter; `boyko_ecs`'s fold reads `take_raised()` at the first drain after boot and emits the code then. So a profiling `W9201` refused before `LogPlugin::build` is not lost — it is emitted at frame 1. This is strictly better than "boot the logger earlier", which is unenforceable across every host. The mechanism is `substrate/mute-leaf-rule`'s report-above-the-leaf, and its three stated costs (reported at the NEXT fold, not at the instant; a condition raised after the last fold is not reported at all; the flag is a bit, so every flag needs exactly one paired counter) apply here unchanged.

**RED for all four** *(S5)*: (a) `warn!` before `enable()` **with a synchronous destination configured** ⇒ the bytes appear on it; restore the `.bss`-zero drop ⇒ no bytes ⇒ red. (b) a severe record after `shutdown()` ⇒ same. (c) a registered `PRE_FLUSH` callback sets a flag; panic ⇒ the flag is set **and** it ran *before* the crash drain; move the call after the drain ⇒ the ordering assertion reds. (d) a deferred `DiagFlag` raised pre-boot appears in frame 1's output; delete the sticky flag ⇒ absent ⇒ red.

---

## Decision 22: `BinarySink` — the only mechanism that raises the ceiling, shipped with a revert clause

The sink writes `{site_id: u16, tsc_delta: u32, len: u16, flags: u8, clock_epoch_lo: u8, payload}` with **no formatting**; `site_id` comes from `SITE_DICT`, a consumer-role-only open-addressed `*const LogSite -> u16` table (4096 entries, 64 KiB `.bss`), with a dictionary record emitted on a `#[cold]` miss and `boyko-W0116` + an inline site record on a full table. `logdec` (a small bin) replays the dictionary and formats offline.

**Every width above is pinned in the session-scale integer audit (Decision 21, `logging/ring-and-statics`), not deferred to the format document** *(fixes M2)*. `docs/LOG-BINARY-FORMAT.md` owns the byte layout and `schema_version`; it does **not** own the session-scale argument, because deferring the widths is what let v3 claim "every integer was audited" while auditing none of these. The two rows that bind this sink's behaviour, restated because they are behavioural and not merely dimensional:

- **`tsc_delta: u32`** is a delta from the file's current **anchor**. A `u32` of raw ticks spans **1.4 s at 3 GHz**, so the sink re-emits an anchor record whenever the delta would exceed `u32::MAX` **or** every 1 s, whichever comes first — and unconditionally after a rotation. **A missed anchor is a decode refusal, never a wrong timestamp.**
- **`site_id: u16`** indexes `SITE_DICT`'s **4096** entries, so the width has 16× headroom over the table. The **table**, not the width, is the limit: on a full `SITE_DICT` the sink emits `boyko-W0116` once and writes an **inline site record** (file/line/fmt spelled out) instead of a dictionary reference, so no record is lost and no id is reused.

The decoder **refuses** a `schema_version` mismatch rather than best-efforting it. Every rotated file re-emits the anchor and the dictionary so it decodes standalone.

**Revert clause (G12c)**: the entire justification is throughput. If `sink_sustained_rate_binary` does not measure ≥ 5× `sink_sustained_rate` in the same sitting, **L13b is reverted**. A format whose only reason to exist is speed must show the speed.

---

## Decision 23: Runtime control with no restart, no lock, and no I/O on the caller's thread

- **Levels / sampling / sync**: a `CAS` on one `CONTROL` byte from any thread. `CONTROL_EPOCH_CTR` is a `Release` counter a UI polls to know it must repaint — an `O(1)` substitute for the change detection Principle 0's refused ECS route would have given. *(S11 naming, stated once: the static is `CONTROL_EPOCH_CTR` and the public accessor is `control_epoch()`; "`CONTROL_EPOCH`" elsewhere names the datum, not a symbol, and it is **not** a clock epoch — `boyko_diag::clock_epoch()` is — nor a flush sequence, which is `FLUSH_SEQ`. `seam/vocabulary` owns the rename set.)*
- **Sink state / filter / floor**: plain byte stores into `SinkSlot` from any thread. A sink acts on the filter it read at the top of its **current** drain, so a change lands within one drain — a stated property, pinned by G13, not hidden.
- **Sink lifecycle (open / close / retarget)**: goes through `SINK_REQ`, a 16-entry `.bss` ring written under `OUT_LOCK`, consumed by the sink thread. **No `open`, no allocation and no syscall ever runs on the requesting thread** — G13b proves it with the per-thread counting allocator. A full queue is `boyko-E0107`, never a silent drop. A channel was rejected: it is an allocation and usually a `Mutex`.
- **`apply_control_spec("net=debug/6!, ecs=off")`** parses a console/env/file spec, applies it with one `control_epoch()` bump, leaves unnamed targets **bit-identical**, and rejects an unknown name with a coded error rather than ignoring it (test 30).

**Capability vs state, as the project rule requires**: a category *exists* because a `LogTarget` (or a dynamic registration) exists — structural. It is *on or off* by a bit in `CONTROL` — state. The rule's substance is honoured at the layer that can afford it; the refusal record in `logging/dispositions` states why `CONTROL` is not an ECS column and what that costs.

**`apply_control_spec` is also the natural home of the enable flag's payload** (S13): whichever route the owner picks for delivering the flag — an env var matching the 28 existing `BOYKO_*` switches, or a new argv parser in `boyko_app` — its *string* is exactly this spec grammar, and `enable(spec)` is `apply_control_spec` plus the one-time work in the move table. The delivery mechanism is an open SCOPE call (`seam/open-owner-calls`); **nothing in this file is blocked on the answer**, because both routes call the same function.

---

## Decision 24: The crash drain CASes the CONSUMER ROLE, not a state that merely correlates with it *(fixes B5)*

**The defect first.** v3 attempted `SINK_STATE.compare_exchange(from, CrashDraining)` for `from ∈ {Exited, NotBooted, Manual}` and called those "the three states in which no sink thread can be inside a drain". Two of them are; **`Manual` is not**. `Manual` does not mean "no consumer is running" — it means the consumer is an *arbitrary user, CLI or test thread that may be inside `drain()` right now*. A panic on any other thread would then CAS `Manual → CrashDraining` and start a **second consumer** over the same lanes, `STAGE`, `SITE_DICT` and `SINK_OUT` — precisely what the `LogLane` SAFETY block forbids. v3's mitigation ("`Manual` documents `drain()` as single-caller and asserts it") constrains manual callers *to one another*, not the crash path, and G14 only tested `Running`, so the hole was untested by construction. It was also reachable in **production**, because `shipping-min` was the `Manual` + crash-sink profile.

**What (corrected).** The role itself is the CAS'd object:

```rust
/// 0 = free; otherwise an opaque token identifying the thread that currently
/// HOLDS THE CONSUMER ROLE. This is the object the Lane SAFETY block's clause 2
/// is about, so it is the object that is CAS'd — not a state that correlates
/// with it.
static DRAIN_OWNER: AtomicU64 = AtomicU64::new(0);
```

Every consumer claims it the same way, and there are exactly four:

| Claimant | When | On failure |
|---|---|---|
| the sink thread (`SinkMode::Thread`) | at the top of every drain pass | re-park; try next pass |
| `drain()` (`SinkMode::Manual`) | on entry | return `DrainResult::Busy` — **not** a `debug_assert`, because a second manual caller is a user error, not a bug in this crate |
| `log_drain_system` (`SinkMode::Scheduled`) | once per frame in `Last` | skip this frame; the records stay in the lanes |
| the crash drainer | panic-hook step 3 | **return without draining** |

The panic hook (chained ahead of the existing hook) writes the panic message synchronously, runs the `PRE_FLUSH` callbacks (step 1.5, S5), then `flush()`. If `flush()` cannot succeed it attempts `DRAIN_OWNER.compare_exchange(0, my_token, AcqRel, Acquire)` **once**. Only on success does it run Algorithm C into the `CrashSink` (a file **opened at `enable()`**, because opening a file inside a panic hook is its own failure mode) and emit `boyko-E0109`. On failure it returns: some other thread holds the role and displacing it would put two consumers on one lane. Termination is a single CAS and a bounded drain; **no wait is added**.

*(S13 moves that file open from `boot()` to `enable()`. The argument is untouched: `enable()` still runs at launch, before the first frame, on the host thread, so the file is still open long before any panic hook could need it. What changes is that a process which never enabled diagnostics never opens the file at all — and its crash drain correspondingly has nowhere to write, which is the same honest boundary `sink_can_accept()` states above.)*

`SINK_STATE` keeps its lifecycle job (`NotBooted`/`Running`/`Exiting`/`Exited`/`Manual`/`Scheduled`) and loses its exclusivity job. `CrashDraining` is deleted as a `SINK_STATE` variant — the fact that a crash drain is in progress is `DRAIN_OWNER != 0`, and there is now one authority for one question.

**The data-race clause this decision exists to make true**, carried here because it is the conclusion the CAS buys: **no lane ever has two consumers** — all four consumers acquire the same CAS'd `DRAIN_OWNER` token. v3 inferred exclusivity from `SINK_STATE ∈ {Exited, NotBooted, Manual}` and was wrong about `Manual`, which is a state in which a consumer may be *running*. The full per-datum ordering table is one object and lives in `logging/ring-and-statics`; it is cited, not restated.

**What it cannot do**: survive `abort()`, `SIGSEGV`, or a guard-page stack overflow — the hook does not run. Stated in E22 and in G14's "cannot claim" column, with the partial mitigations named (the per-target sync bit **and its real durability bound**, `flush_interval_ms`, and a crash file that at least exists and carries the session header). It also cannot drain when another consumer is mid-pass; in that case the records that consumer has already staged are written by *it*, and the rest are lost — which is the honest outcome and is why the loss is counted rather than assumed away.

---

## Decision 25 (runtime half): `LogRuntimePreset` — five presets, and the compile axis is NOT one of its columns *(re-cut by S9; consumer discipline by B8)*

**Two axes, and v3 conflated them.** `GLOBAL_CEILING` and the lane count are **compile-time consts**; a struct chosen by the host at run time cannot deliver either, so v3's table promised something its own type could not do. **S9 separates them, and the compile axis is the seam's** (`seam/build-axis`): the `BOYKO_PROFILE` env var, the one `crates/boyko_diag/build.rs` that reads it, the profile→const table, the `custom` rule, the CI-leg count and G16's second and third REDs all live in `SEAM.md`. What is owned **here** is the runtime axis and nothing else.

**The runtime axis is `LogRuntimePreset`** (v3 called it "the `LogConfig` profile"), which selects sinks, rotation, sampling, sink mode and census policy. It has **no `GLOBAL_CEILING` column**.

| `LogRuntimePreset` | Sinks | Rotation | Sampling | `SinkMode` | Census | Intended for |
|---|---|---|---|---|---|---|
| `Dev` | console + file | `Rotation::NONE` | off | `Thread` | `OnFlush` | engine work, benches, goldens |
| `Editor` | console + file | on | off | `Thread` | `Interval(10)` | long editor sessions |
| `Shipping` | binary + crash | on | opt-in | `Thread` | `OnShutdown` | a released title |
| `ShippingMin` | crash only | on | opt-in | **`Scheduled`** *(was `Manual` — B8)* | `OnShutdown` | a title that wants no **resident** diagnostics thread |
| `Off` | none | — | — | — | — | G2's leg |

**The default is a default, not a coupling**: a `shipping` build may select `LogRuntimePreset::Dev` at run time, and the header must make that legible — which is why it prints **three independent facts**, `build_profile=… runtime_preset=… ceiling=…`, and not one profile name. The 128-bit **`boyko_diag::SessionId`** (one mint, shared with the profiler's artifact header, S11) appears beside them, so an uploaded log and an uploaded artifact identify the same session.

**`ShippingMin` has a consumer** *(B8)*. `SinkMode::Scheduled` puts the drain in `log_drain_system` under `DRAIN_OWNER`, once per frame, on the frame thread (Decision 10). What the profile actually buys is **no resident diagnostics *thread***, which is what the owner asked for; what it costs is a bounded per-frame drain and a hole around boot and shutdown, both stated in Decision 10. *(An owner-facing SCOPE question remains, recorded once in `seam/open-owner-calls`: the profiler's `Always` tier still writes a telemetry stream synchronously on the dispatcher in this profile, so a title that chose `shipping-min` to avoid a resident diagnostics thread still pays a per-window `write_all`.)*

**S13's interaction with the preset table, stated because it is easy to get backwards.** A preset says *what is configured when diagnostics are on*. It does not say *whether they are on*. `Off` is a preset that configures nothing; a **flag-off run of any other preset** configures nothing either, because `enable()` never ran and no sink slot was ever opened. The two reach the same resident cost by different routes, and only one of them can be turned back on without relaunching with a different config — which is the entire point of keeping the runtime axis.

---

## Decision 26 (transport half): the handoff is a SPECIFIED structure, not a word *(fixes B2)*

**The transport, which v3 named three times and defined nowhere.** v3 wrote "push formatted lines to the ECS handoff ring" (Algorithm C), "fed by `log_drain_system` in `Last`, from the sink's handoff" and referenced it again in the public API — with **no type, no capacity, no ordering, no overflow accounting, no budget row and no `Send`/`Sync` argument**. Every claim about the reader rests on it, and an undefined cross-thread queue is exactly the object this campaign's defects live in.

*(The reader surface — `LogRing::since`, `RingFilter`, `LogRingIter::skipped`, `log_drain_system` in `Last`, and the property that a record is never visible before the drain that consumed it — is `logging/game-facing-surface`. What is owned here is the ring that feeds it.)*

```rust
/// SPSC byte ring carrying FORMATTED LINES from the consumer role to the ECS.
/// Deliberately the SAME SHAPE as `LogLane` — same header, same MASK
/// arithmetic, same PAD/wrap rule, same Release/Acquire cursor pair — so it
/// introduces NO new protocol, only a new instance. Reusing the rule is why
/// its correctness argument is one sentence instead of a section.
#[repr(C, align(64))]
pub(crate) struct HandoffRing {
    write: AtomicU32, _p0: [u8; 60],                 // producer line: the consumer role
    read:  AtomicU32, _p1: [u8; 60],                 // consumer line: log_drain_system
    lost:  AtomicU64, lost_bytes: AtomicU64, _p2: [u8; 48],   // LossClass::Sink
    buf:   UnsafeCell<[MaybeUninit<u8>; HANDOFF_BYTES]>,
}
static ECS_HANDOFF: HandoffRing = HandoffRing::NEW;
pub const HANDOFF_BYTES: usize = if SHIPPING { 64 * 1024 } else { 256 * 1024 };
const _: () = assert!(HANDOFF_BYTES.is_power_of_two());
```

| Property | Value | Why |
|---|---|---|
| **Producer** | whoever holds `DRAIN_OWNER` — the sink thread, `drain()`, `log_drain_system` under `Scheduled`, or the crash drainer. **Exactly one at a time, by the Decision 24 CAS** | this is what makes the ring genuinely SP, rather than "SP by convention" |
| **Consumer** | `log_drain_system` **only**, holding `ResMut<LogRing>` — so the scheduler's exclusivity analysis is the SC proof | one writer of `LogRing`, which is what B1's `Send`/`Sync` argument needs |
| **Capacity** | 256 KiB `dev` / 64 KiB `shipping` (Decision 3's matrix, `logging/ring-and-statics`). At ~120 B per formatted line that is ~2 200 / ~550 lines per frame | a frame that formats more than that is already lossy at the sink |
| **Ordering** | `write.store(Release)` publishes; `read.store(Release)` after the ECS copy frees. Identical to `LogLane` | the payload here is plain text with no pointers, so the provenance clause does not apply |
| **Overflow** | the producer **refuses**, counts into `lost`/`lost_bytes` as `boyko_diag::LossClass::Sink`, and the drain emits **one `boyko-W0117`** per drain carrying the count. `LogCensus.lossy` is set. **Never silent** | the byte sinks already have the record; only the *in-frame view* is short, and a reader must be able to tell |
| **Allocation** | **none, ever** — `.bss`, compile-time extent, S12's rule | |
| **Presence** | only when `LogConfig.ecs_ring` is set. Absent in `ShippingMin`. **Under S13, also never touched on a flag-off run** — the extent is reserved, the pages are not committed | no ECS reader, no ring |

```
// SAFETY (manual Sync for HandoffRing):
//   1. WRITE side: only the holder of DRAIN_OWNER writes `buf`/`write`. The
//      role is a single CAS'd token (Decision 24), so two producers is
//      unrepresentable — the same clause LogLane's SAFETY block makes about
//      `owner`, one level up.
//   2. READ side: only `log_drain_system` reads `buf` and writes `read`, and
//      the scheduler grants it `ResMut<LogRing>` exclusively, so two consumers
//      is unrepresentable.
//   3. Visibility: bytes written before `write.store(Release)` are visible to
//      the reader's `Acquire`; the reader never reads past its observed `w`
//      and never advances `read` over bytes it has not copied into `LogRing`.
//   4. No pointers cross: the payload is UTF-8 bytes already formatted by the
//      consumer role, so the `LogLane` provenance clause has no analogue here.
```

**The stated bound** is "sink park interval + one frame" (≤ 2 frames in practice) under `Thread`, and **one frame** under `Scheduled` (the drain and the ECS copy are the same system). G15 cannot claim tighter. A per-frame **`frame_epoch` record** *(renamed from `EPOCH` — S11, three meanings collided; `seam/vocabulary`)* lets a reader attribute every record to exactly one frame; a record emitted *during* the drain is attributed to the next frame, and test 29 asserts that rather than assuming it.

---

## Data structures — the sink and lifecycle statics

```rust
// ────────────────────────── boyko_log/src/sink/*.rs ───────────────────────────

/// Sink KIND is fixed at boot; STATE and FILTER are runtime-mutable by a byte
/// store from any thread. Lifecycle ops (open/close/retarget) go through
/// SINK_REQ so that no syscall and no allocation ever runs on a caller's
/// thread (Decision 23, G13).
#[repr(C, align(64))]
struct SinkSlot {
    state:  AtomicU8,          // Off | Active | Paused | Closing | Faulted
    floor:  AtomicU8,          // minimum level this sink accepts
    kind:   SinkKind,          // Console | File | Binary | Callback | Crash
    filter: [AtomicU64; 4],    // 256-bit target mask; bit i == target i accepted
    _pad:   [u8; 26],
    // The handle / cursor / rotation state is owned by the SINK THREAD alone.
}
pub const MAX_SINKS: usize = 8;
static SINKS: [SinkSlot; MAX_SINKS];

#[repr(C, align(64))]
struct SinkReq { seq: AtomicU32, op: u8, slot: u8, path_len: u16, path: [u8; 256] }
static SINK_REQ: [SinkReq; 16];          // full ⇒ boyko-E0107, never silent
static SINK_REQ_HEAD: AtomicU32; static SINK_REQ_TAIL: AtomicU32;

// Lifecycle / completion. `SINK_STATE` no longer carries the exclusivity proof
// — `DRAIN_OWNER` does (Decision 24) — so it has exactly one job.
static SINK_STATE:  AtomicU8;   // NotBooted | Running | Exiting | Exited | Manual | Scheduled
static SINK_EXITED: AtomicBool;
static FLUSH_SEQ:   AtomicU32;  // RENAMED from FLUSH_REQ's "epoch" (S11)
static FLUSH_ACK:   AtomicU32;
static DRAIN_OWNER: AtomicU64;  // 0 = free; else the consumer role's token (Decision 24)

// The synchronous channel's lock (Decision 9c).
static OUT_OWNER:     AtomicU64;   // 0 = free; else an opaque thread token
static OUT_STEALS:    AtomicU32;
static OUT_REENTRANT: AtomicU32;

// The pre-flush callback seam (S5). Claimed by CAS; a ninth is boyko-E0118.
static PRE_FLUSH: [AtomicPtr<()>; 8] = [const { AtomicPtr::new(null_mut()) }; 8];

// The three consumer-role scratch buffers. ALL THREE ARE `.bss` STATICS, not
// heap — v2 left `STAGE_BYTES`'s backing store unspecified, and "no `Vec`/`Box`
// in any SIGNATURE" is narrower than the claim a reader takes from it (F25).
// They are counted in Decision 3's budget matrix (`logging/ring-and-statics`).
static STAGE:     UnsafeCell<[u8; STAGE_BYTES]>;       // 256 KiB — Algorithm C
static SITE_DICT: UnsafeCell<[SiteDictEntry; 4096]>;   // 64 KiB — binary sink only
static SINK_OUT:  UnsafeCell<[u8; 1 << 20]>;           // 1 MiB — binary write buffer
// SAFETY: all three are touched only by the thread currently holding the
// consumer role — the sink thread, or the crash-draining thread after the
// DRAIN_OWNER CAS proved no other consumer can be inside a drain (Decision 24).
// (v3's comment named the SINK_STATE CAS here; B5 replaced the object, and this
// SAFETY text moves with it — a SAFETY block that names a superseded proof is
// worse than none, because it reads as verified.)

// ────────────────────── boyko_log/src/sink/ecs.rs (B2) ────────────────────────
// `HandoffRing` / `ECS_HANDOFF` — the sink -> ECS transport, specified in full
// in Decision 26 above (layout, capacity, ordering, overflow, budget row,
// SAFETY). Same shape and same wrap rule as `LogLane`: no new protocol, one new
// instance. Single producer = the DRAIN_OWNER holder; single consumer =
// `log_drain_system` holding `ResMut<LogRing>`.
```

**All of the above are `.bss` with compile-time-const extents**, so they obey S12's rule by construction and not by exemption (`substrate/never-freed-storage`). Under S13 that has a second consequence: **on a flag-off run none of them is written**, so each costs address space and no physical page. The claim's limit is the one `substrate/section-report` states — absence of raw data in the image is proved; loader behaviour for an untouched page is not, and is not claimed.

---

## Public API — the lifecycle and control slice

```rust
// ── runtime control (any thread, no lock, no restart — Decision 23) ───────────
pub fn target_control(id: TargetId) -> TargetControl;
pub fn set_target_control(id: TargetId, ctl: TargetControl);   // CAS, preserves siblings
pub fn set_target_level(id: TargetId, lvl: Level);             // CAS, preserves shift+sync
pub fn control_epoch() -> u32;                                 // O(1) repaint signal
pub fn apply_control_spec(spec: &str) -> Result<u32, ControlSpecError>; // "net=debug/6!"

pub fn sink_state(slot: u8) -> SinkState;
pub fn set_sink_state(slot: u8, s: SinkState);                 // byte store, any thread
pub fn set_sink_filter(slot: u8, targets: &[u64; 4], floor: Level);
pub fn request_open_file(slot: u8, path: &str) -> Result<(), SinkReqError>;  // E0107 if full
pub fn request_close(slot: u8) -> Result<(), SinkReqError>;

// ── lifecycle ─────────────────────────────────────────────────────────────────
pub struct LogConfig {
    pub sink_mode: SinkMode,            // Thread (default) | Manual | Scheduled (B8)
    pub console:   Option<ConsoleSink>, // stderr's own handle; colour/level floor (S7)
    pub file:      Option<FileSink>,    // path, rotation
    pub binary:    Option<BinarySink>,  // path, rotation, flush_interval_ms
    pub crash:     Option<CrashSink>,   // path; OPENED AT `enable()` (Decision 24 + S13)
    pub callback:  Option<CallbackSink>,// extern "C" fn(&FormattedRecord, *mut ()) + ctx
    pub ecs_ring:  bool,                // enables ECS_HANDOFF + LogRing (D26)
    /// `write_oracle_line` also `sync_data()`s the crash handle. ~0.1-10 ms per
    /// record; OFF by default, because a sync bit that also fsync'd would
    /// serialise the frame on the disk rather than on the format (B9).
    pub sync_durable: bool,
    pub census:    CensusPolicy,        // OnFlush (dev) | OnShutdown | Interval(secs)
    pub control_source: ControlSource,  // None | Env | File(&'static str)
    pub default_controls: [TargetControl; MAX_TARGETS],
}
/// RENAMED from "the `LogConfig` profile" (S9) and it has NO ceiling column:
/// `GLOBAL_CEILING` and `LANE_COUNT` are compile-time consts from
/// `BOYKO_PROFILE` (the seam's axis), which a runtime preset cannot deliver.
/// The header prints `build_profile` / `runtime_preset` / `ceiling` as three
/// independent facts.
pub enum LogRuntimePreset { Dev, Editor, Shipping, ShippingMin, Off }
impl LogRuntimePreset { pub fn config(self) -> LogConfig; }

pub struct Rotation { pub max_bytes: u64, pub keep: u8 }   // Rotation::NONE = v2 behaviour
pub enum LogBootError { AlreadyBooted, TargetIdCollision { id: u16, a: &'static str, b: &'static str } }
/// S13: sink opening moved off `boot()`, so `SinkOpen` moved with it.
pub enum LogEnableError { NotBooted, AlreadyEnabled, SinkOpen(std::io::Error), Spec(ControlSpecError) }
pub enum FlushResult { Flushed, NoConsumer, TimedOut }
/// `Busy` is why `drain()` returns something: a second manual caller is a USER
/// error, not a bug in this crate, so it is refused rather than asserted (B5).
pub enum DrainResult { Drained { records: u32 }, Busy, NoLanes }

pub fn boot(cfg: LogConfig) -> Result<(), LogBootError>;   // no handle (Decision 12);
                                                           // spawns nothing, opens nothing (S13)
pub fn enable(spec: &str) -> Result<(), LogEnableError>;   // S13: the ONE place one-time work runs
pub fn shutdown();                                         // idempotent; no-op if never enabled
pub fn flush() -> FlushResult;                             // never waits on a dead consumer
pub fn drain() -> DrainResult;                             // claims DRAIN_OWNER, or Busy
/// Registers an `extern "C" fn()` called by `flush()`, by `shutdown()` and by
/// the panic hook BEFORE the crash drain (S5). Eight slots; a ninth is
/// `boyko-E0118`. A registrant must not allocate, must not lock, must not
/// touch the `World`, and must do at most one `write_all`.
pub fn register_pre_flush(f: extern "C" fn()) -> Result<(), PreFlushFull>;
pub fn session_id() -> boyko_diag::SessionId;              // ONE mint (S11)
pub fn census() -> CensusIter<'static>;                    // Measured / Unproven per target
pub fn name_current_thread(name: &'static str);            // cosmetic, cold, once
pub fn write_oracle_line(prefix: &str, body: &str);        // the synchronous fan-out (D9c)
```

*(`name_current_thread` is **restored** here: it is `docs/LOGGING-SYSTEM-PLAN.md:1650`, it sits in the very block this file's carve header claims to carry whole, and it was dropped at the split without a word — while every other change to this block was announced (`enable` added, `SinkOpen` migrated to `LogEnableError`, `explain` moved to `logging/registry-and-walker`). A silent deletion from a slice the header promises to carry is indistinguishable from an oversight, and this file announces its removals — `report!`, `crates/boyko_log/src/tsc.rs`, the `CrashDraining` variant — by name. It does **not** conflict with S3: the substrate lane index is the machine-readable thread identity in every artifact, and this is the human-readable one an OS debugger shows, which no lane number can supply.)*

No `Vec`, `Box<dyn>`, `HashMap` or internal type appears in any signature. The callback seam is an `extern "C" fn` + ctx, so it crosses a dylib boundary with no vtable and no allocation.

---

## Algorithms

### C. Drain — staged copy BEFORE the free *(fixes B1)*

```
0. CLAIM THE CONSUMER ROLE (B5). Every consumer does this, identically:
   if DRAIN_OWNER.compare_exchange(0, my_token, AcqRel, Acquire).is_err() {
       return Busy    // sink: re-park. Manual: DrainResult::Busy.
   }                  // Scheduled: skip this frame. Crash: do not displace.
   // Released by an RAII guard, so an unwind inside the drain does not strand
   // the role — the same shape as OutGuard (D9c).

STAGE_BYTES = 256 KiB (a `.bss` static, consumer-role-owned, reused every drain — F25)

drop_tally = {lanes: 0, records: 0, bytes: 0, by_class: [0; LOSS_CLASSES]}
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
      s += hdr.len; r = r.wrapping_add(hdr.len)
  read.store(r, Release)          // ← ONLY NOW is the ring space published free
  fold this lane's LossCell[..] / sampled_out into drop_tally, TARGET_STATS and
      LogStats; clear each with fetch_sub(observed) — NEVER store(0), because a
      producer increment between the load and the clear would be lost (S8)
  reclaim via boyko_diag if RETIRING && r == w
if drop_tally.records > 0 { synthesise ONE boyko-W0102 for the whole drain,
      carrying lanes_affected / records / bytes / the LossClass breakdown }
      // one per DRAIN, not one per lane: 125/s instead of ~16 000/s (F24)
sort_unstable_by_key(tsc)         // over preallocated 16 B triples; no allocation
for each staged record, for each ACTIVE sink whose filter accepts (target, level):
    text sinks : (*site.decode)(stage + s + HEADER_BYTES, len, &mut fmt)  // reads STAGING
                 // the sink renders `clock_epoch_lo` beside the timestamp (S4)
    binary sink: site_id = SITE_DICT lookup (#[cold] miss ⇒ dictionary record;
                 FULL table ⇒ W0116 once + an inline site record — M2)
                 append {site_id, tsc_delta, len, flags, clock_epoch_lo, payload}
                 // NO formatting (D22); anchor re-emitted at 1 s or u32 overflow
one write_all per byte sink per fill
if cfg.ecs_ring {
    push each formatted line into ECS_HANDOFF (D26) — the SAME wrap rule as a
    LogLane; on refusal, LossClass::Sink += 1 and ONE boyko-W0117 per drain.
    // The sink NEVER touches LogRing or LogCensus. That is what makes B1's
    // Send/Sync argument true rather than merely written.
}
release DRAIN_OWNER (RAII)
```

**The `fetch_sub(observed)` clear is the CONSUMER side of the fold only.** The producer side is `substrate/loss-fold`'s open blocker Q2: an owner increment is `load; add; store`, and a consumer `fetch_sub` landing between that load and that store is overwritten, so the one loss event is counted twice. That is an architect call carried by the substrate, and until it is answered `boyko_diag` ships `loss.rs` **without** `fold_into`. This algorithm's clear step is written against the closed half and must not be read as a claim about the open half.

**Fan-out is inside ONE drain.** Every sink reads the same staging arena; there is never a second consumer of a lane. That is what makes "text + binary + crash simultaneously" cost one pass, and it is why the refusal record rejects a second sink thread.

**Why the order changed.** v1 advanced `read` at step 3 and decoded at step 6 from an `offset` **into the ring** — bytes the producer was licensed to overwrite in between. The sink would then read a torn header and call `decode` through 8 arbitrary bytes reinterpreted as a function pointer. v1's tests could not see it: both the ordering test and the overflow test drive a quiesced producer. The staged copy makes the window structurally absent, and adds a bound: a drain never stages more than `STAGE_BYTES`, so a hot lane is drained across several passes rather than in one unbounded burst.

**Provenance.** The header — the only field carrying a pointer — is moved by a **typed** `read_unaligned`/`write` pair, never by a byte memcpy, so `site`'s provenance round-trips by construction rather than by relying on per-byte provenance tracking. Payloads are pointer-free POD and move by `copy_nonoverlapping`. Gated by Miri under Tree Borrows (test 14).

- **Complexity** O(R log R) per drain for R records, entirely off the frame thread (except under `Scheduled`, where it is bounded and on it by design).
- **Stated limitation** cross-lane ordering is *approximate*: a record written after lane A's snapshot may carry an earlier `tsc` than one already staged from lane B. Inherent to any non-blocking merge (Quill has the same property) and printed in the sink's header line.

### D. `flush`

```
0. run every registered PRE_FLUSH callback (S5) — before anything else, so a
   registrant's last window reaches disk even if step 1 short-circuits
1. match SINK_STATE.load(Acquire) {
       NotBooted | Manual | Scheduled | Exited => return FlushResult::NoConsumer,
       Running | Exiting => {}
   }
   // `Scheduled` joins the short-circuit set: its consumer is the schedule,
   // which `flush()` cannot unpark and must not impersonate (B8 + B5).
   // S13: a flag-off run never leaves `NotBooted`, so this is also the path
   // every un-enabled process takes — at the same zero cost.
2. seq = FLUSH_SEQ.fetch_add(1, AcqRel) + 1        // RENAMED from FLUSH_REQ's
                                                   // "epoch" — S11, three meanings
3. unpark(sink)
4. spin_backoff until FLUSH_ACK.load(Acquire) >= seq, deadline = now + 2 s
5. on timeout: write_oracle_line("boyko-E0105: log flush timed out"); TimedOut
   // write_oracle_line is BOUNDED (≤ 50 ms, then steal) — Decision 9c. v2's
   // bounded wait terminated in an UNBOUNDED one (F8).
```

Step 1 is what keeps `#[should_panic]` tests in an unbooted binary at zero cost. Step 5 is non-negotiable: the profiling audit's central finding is that an unbounded blocking wait converts an instrumentation gap into an unkillable hang, and that this repository has no kill-after-timeout pattern to borrow (`crates/boyko_app/tests/vb_bench_totality_gate.rs:48-49`, quoted above). This design does not add a second one.

### E. Crash drain *(Decision 24)*

```
panic hook (chained ahead of the existing hook; INSTALLED BY enable(), not boot() — S13):
1.  write_oracle_line(panic message)                      // synchronous, bounded,
                                                          // durable fan-out (B9)
1.5 run every registered PRE_FLUSH callback (S5)          // BEFORE the crash drain;
                                                          // no alloc, no lock, no World
2.  match flush() { Flushed => return, _ => {} }
3.  if DRAIN_OWNER.compare_exchange(0, my_token, AcqRel, Acquire).is_ok() {
        // The consumer role is now EXCLUSIVE BY POSSESSION, not by inference
        // from a state that merely correlates with it (B5). v3 CAS'd
        // SINK_STATE out of {Exited, NotBooted, Manual} — and `Manual` means
        // an arbitrary thread may be INSIDE drain() right now, so v3 could
        // start a second consumer over these lanes in the one profile
        // (`shipping-min`) that shipped it.
        run Algorithm C over every lane, text sink = CRASH sink only
        write_oracle_line("boyko-E0109: crash drain took the consumer role")
        release DRAIN_OWNER; SINK_STATE.store(Exited, Release); return
    }
4.  // Someone else holds the role — a live sink thread, a manual drain(), or a
    // scheduled drain. DO NOT displace it: two consumers on one lane is what
    // the LogLane SAFETY block forbids. Whatever that consumer has already
    // staged, IT will write; the rest is lost and counted. flush() already
    // timed out and said so (E0105).
    return
```

- **Termination**: step 3 is a single CAS; step 4 returns. No wait is added.
- **What it cannot do**: survive `abort()`, `SIGSEGV`, or a guard-page stack overflow — the hook does not run. Written down in E22 and in G14's "cannot claim" column rather than mitigated by a heuristic.
- **On a flag-off run there is no hook at all** (S13), which is what G2 leg (c) observes *behaviourally* — `take_hook()` is destructive and returns an unidentifiable `Box<dyn Fn>`, so the presence of a hook cannot be inspected and must be provoked.

---

## Multithreading — the consumer half

The per-datum sharing/ordering table is **one object** and is carried whole in `logging/ring-and-statics`; restating rows here is precisely how two files come to disagree about a memory ordering. What is owned here is the three consumer-side arguments the table's rows rest on:

1. **Drain exclusivity.** All four consumers — the sink thread, `drain()`, `log_drain_system` under `Scheduled`, and the crash drainer — acquire the same CAS'd `DRAIN_OWNER` token (`AcqRel` on success, `Acquire` on failure, RAII `Release`). It is uncontended in every normal configuration and contended only when a panic races a drain. `SINK_STATE` no longer carries the exclusivity proof, so `CrashDraining` is deleted from it and one question has one authority.
2. **The sink thread's park policy is not a synchronisation mechanism.** `park_timeout(0)` / `park_timeout(8 ms)` is a *throughput* policy; correctness rests entirely on the `Release`/`Acquire` cursor pair. Producers never unpark — that would be a syscall per record — so a record's *visibility* is never a function of whether the sink was awake. `flush()` and `shutdown()` unpark; nothing else does.
3. **The crash path's role CAS is the whole of its safety argument.** Termination is the CAS plus a bounded drain; failure is a return, not a wait. The staged-copy ordering of Algorithm C (`read.store(r, Release)` only after the bytes are in `STAGE`) is what makes the crash drainer's read of a live lane sound at all, and it is the same rule the sink thread obeys.

`STAGE`, `SITE_DICT` and `SINK_OUT` are consumer-role-owned by exactly the argument in clause 1: their SAFETY block names the `DRAIN_OWNER` CAS, not the superseded `SINK_STATE` CAS.

---

## Free when not enabled — what this file owes S13

The full requirement, its two axes, the three-row honest cost table and gate `GJ1` are `seam/free-when-off`. This section records only the changes that land **in this file**, so a reader who never opens `SEAM.md` still gets correct behaviour.

**`boot()` becomes a pure struct-fill.** It validates the config, resolves target ids, publishes the sink *kinds*, and returns. It:

- does **not** spawn the sink thread (Decision 10) — that is `enable()`;
- does **not** install the process-global panic hook or register `PRE_FLUSH` (Decision 12, Algorithm E) — that is `enable()`;
- does **not** open the crash file, the log file or the binary file (Decision 24, Decision 9c's fan-out table) — that is `enable()`;
- does **not** calibrate the clock (Decision 11 / `substrate/clock-source`) — the first of `boyko_log::enable()` / `Profiler::arm()` to run does, and `calibrate()` is already idempotent and CAS-guarded, so "whichever runs first" needs no new mechanism;
- does **not** touch a lane buffer, a `RATE` slot, `SINK_OUT`, `STAGE`, `SITE_DICT` or `ECS_HANDOFF`.

**`enable(spec)` does all of it**, at launch, before the first frame, on the host thread, where a syscall and a 20 ms calibration window are free of both hot-path and frame-time concerns. `disable()` is its inverse for a session that turns diagnostics back off; it stops the consumer and closes the handles but does **not** reclaim `.bss`, because `.bss` is never freed (S12).

**What this buys and what it does not.** Boot work goes to **zero, and that one really is zero** — observable, and observed: G2 leg (b)'s OS-thread-count probe (which carries its own control, so a probe returning a constant reds before it can certify anything) and leg (c)'s behavioural panic-hook probe both re-point from the `off` *build profile* to the flag-off *run*. Memory goes to **address space, not resident** — with the flag off `claimed_lanes == 0`, no lane buffer is written, and `SINK_OUT`/`STAGE`/`SITE_DICT`/`ECS_HANDOFF` are untouched. **Per-site instruction cost does not go to zero and cannot**: the gate byte still has to be read in order to be a gate. Only the compile-time ceiling deletes the site and its operands. That sentence is `seam/free-when-off`'s and is repeated here in those words because this is the file where someone would otherwise write "and then it costs nothing".

**One number in this file is re-cut.** Decision 10's retained window (≈ 32 800 records across 80 lanes) and Decision 26's 256 KiB / 64 KiB handoff are **RESERVED extents**, not resident cost. With the flag off, resident is 0 for both. With the flag on, they are as tabled. The reserved-extent totals themselves live in `logging/ring-and-statics`' budget matrix and the joint figure in `seam/joint-cost`; nothing here restates them.
