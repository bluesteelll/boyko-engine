# Architecture: Engine Logging & Diagnostics (`boyko_log`)

> Target file: `docs/LOGGING-SYSTEM-PLAN.md`. Status: **DRAFT v3 — revised against the second-pass review (verdict REJECTED, findings F1-F26) AND the owner's scope extension: the logger is for GAMES as well as for the engine.** Findings disposition and scope-extension disposition at the end.

## Changelog v2 → v3

v3 folds **two** inputs at once. They are kept separable on purpose: a reader who only cares about the defects can read §Findings disposition (v2 → v3), and a reader who only cares about the new audience can read §Scope-extension disposition. Where the two collide, the collision is named in §Audience conflicts and decided there.

**Input 1 — the second-pass review (REJECTED, 26 findings).** Two of them are outright correctness bugs and are fixed by arithmetic and by a type size, not by prose:

- **F6** — `free = limit - (w - read_cached)` **underflows in `u32`** exactly in the state the Error reserve is designed to produce, and the producer then overruns live ring bytes. Fixed by reformulating admission control as `avail = CAPACITY - used` (an induction that cannot go negative) and applying the reserve with `saturating_sub` (§Decision 5, §Algorithms A6). Gate **G17** exists solely to red on the old arithmetic.
- **F7** — `LogRing`'s `VmColumn<LogLine>` **panics at construction**: `crates/boyko_ecs/src/ecs/memory/vm_column.rs:144-149` asserts `COMMIT_GRANULE % size_of::<T>() == 0`, `COMMIT_GRANULE = 64 KiB` (`crates/boyko_ecs/src/ecs/constants.rs:7`), and v2's `LogLine` was 12 bytes. `LogLine` is now **16 bytes, `Copy`, with a `const _: () = assert!` beside the definition** so a future field addition fails the build instead of the plugin (§Data structures).

Four more were "a gate that cannot fail" in a new costume — the campaign's own recurring defect — and each is either given a showable red state or **deleted**: test 17 (F1), G7's negative leg (F2), P1 (F3), G2's thread/hook legs (F4), registry checks 3/6 (F5). `OUT_LOCK` grows a **bounded, re-entrancy-aware, unwind-safe** protocol (F8) and is **not** registered in `docs/HOT-PATH-EXCEPTIONS.md`, because registering it **reds CI** (F9 — `scripts/check_hotpath_exceptions.py:337-341` matches registry rows against `#[allow(clippy::disallowed_types)]` counts per file, and an atomic carries none).

**Two findings are REFUTED with evidence, and one is refuted in part** — see §Findings disposition. v2 said "Refuted: none"; that was a disposition, not a virtue, and it is not repeated for its own sake.

**Input 2 — the scope extension (games, not just the engine).** Dynamic targets minted from data (Decision 18), downstream code tables (Decision 19), game-defined POD values (Decision 19b), per-target sampling (Decision 20), a session-scale integer audit (Decision 21), a binary sink that does not format (Decision 22), runtime sink/level control with no restart and no lock (Decision 23), a crash drain (Decision 24), four shipping profiles (Decision 25), and the in-frame reader surface a `boyko_ui` HUD and a telemetry reducer consume (Decision 17, Decision 26). Seven asks are **refused with reasons** rather than designed (§Refused).

**What did NOT change, because the review said to keep it:** deferred formatting, the POD record + `&'static LogSite`, `.bss` statics with `Off == 0`, the SPSC lane, `report!` as a separate synchronous channel, B1's staged-copy drain, B3's shared wrap rule, B8's withdrawal of the validation migration, N30's honesty about `W0101`, and M23's tidy-test-primary enforcement.

### Answers to the first review's eight questions (carried forward, still accurate)

1. **Drain order** — the drain copies each record out of the ring into a staging arena, *then* advances `read`, *then* sorts, *then* decodes from staging. `read` never advances over bytes the sink still intends to read. (§Algorithms C)
2. **Encoded length** — `LogArgs::encoded_len(&self) -> usize`, a runtime method that const-folds for all-fixed tuples. `&str` encodes as `u16` length + bytes. `fmtv` no longer exists; a `Display` is rendered by `dsp!` into a caller-owned stack buffer **in argument position**, so the ring is never open while user code runs, and an overrun is a truncation of a `&str` that has already been produced. (§Decision 1a, §Decision 13)
3. **Wrap** — records never straddle. Deterministic shared rule: `LANE_BYTES - off < HEADER_BYTES ⇒ both sides wrap`; otherwise a PAD record (null `site`, `len` = tail) consumes the tail. (§Algorithms A6)
4. **Re-entrancy** — forbidden *structurally*: nothing between lane acquisition and the `Release` store can call user code, because argument encoding is over already-materialised POD and `&str`, and `LogPod::fmt_pod` runs on the sink. A re-entrancy `debug_assert` guard backs it; test 24 asserts it. (§Decision 13, §Decision 19b)
5. **Sink rate** — stated as a design number (≥ 500 K records·s⁻¹ **aggregate**, text sink, at the default geometry) and **gated at L3** by `sink_sustained_rate`, which must show the drop knee. (§Decision 10, §Metrics)
6. **Perturbation** — ABBA-counterbalanced logger-on/logger-off with an interleaved zero control in the same sitting. **v3 changes the instrument**: the channel is a headless schedule bench, not windowed frame time, which FIFO clamps (§Metrics, gate P1, F3).
7. **`[vk-validation]`** — stays synchronous. The migration is withdrawn, **and so is v2's remaining one-line edit** (§Decision 9b, F12).
8. **Handle / flush-without-consumer** — `LogHandle` is deleted; `boot()`/`shutdown()` are free functions over process-lifetime statics. `flush()` reads `SINK_STATE` first and returns `FlushResult::NoConsumer` immediately when nothing can ever acknowledge. There is no `join`-with-timeout because std does not provide one; shutdown observes a sink-exited atomic with a bounded spin and then **detaches**. (§Decision 12)

---

## Goal

Replace the workspace's ad-hoc diagnostic output — **179 raw occurrences across 36 files** under `crates/*/src/**` (reproduced this session: `grep -rEn 'println!|eprintln!|print!\(|eprint!\(' --include='*.rs' crates/*/src` ⇒ 179 / 36; `crates/boyko_shaderdsl/src/bin` ⇒ 58) — 5 hand-rolled `AtomicBool` once-latches, and 9 ad-hoc `boyko-####` codes in three incompatible text formats, with **one in-house subsystem** that obeys the engine's own principles.

**The census arithmetic, stated so it cannot drift again** *(fixes F18)*. v2 opened with "83 shipping print sites". Its own ledger says 179 − 58 (CLI bins) − 23 (in-`src` `#[cfg(test)]`) = **98**. Neither number is a *site* count: 179 is a **raw occurrence** count over unstripped text, and it includes prose — `crates/boyko_app/src/runner.rs:560-561` mentions `eprintln!` twice inside a comment. Two numbers are therefore carried, both named:

- **179 / 36** — the reproducible raw-grep census. Its only job is to be re-derivable by one command.
- **The walker's site count** — macro invocations after comments, string literals and `#[cfg(test)]` regions are stripped (§Decision 6's walker). It is ≤ 98 and it is the migration denominator. **L8c's exit criterion is defined over the walker's count, never over the grep's**, so a comment mentioning `eprintln!` can never be driven into `print_allowlist.txt` to make a gate go green.

**Two audiences.** The engine is audience one: a fixed set of subsystems, known at compile time, where the right answer to "should this site cost anything when disabled?" is *nothing at all, not even a load*. A **game built on this engine** is audience two: hours-long sessions, categories that may be named by data or by a mod, a console that toggles verbosity without a restart, a support-facing log, and gameplay code that wants to read its own diagnostics in-frame. The two are not the same customer, and §Audience conflicts names the five places where they pull in opposite directions and decides each with the cost to the losing side.

**Functional**
- Any thread — Chase-Lev worker, dispatcher, OS/window thread, asset I/O, a script VM, a mod — emits a diagnostic without a lock, an allocation, or a syscall.
- Every `Warn`/`Error` carries a registry code that is documented, uniquely numbered, mechanically proven non-orphan, and explainable. This holds for game and mod code too (Decision 19); there is no relaxed tier.
- **Loss is counted and reported; never silent — and *policy* is reported as policy, not as zero** *(fixes F10)*. Three quantities are kept apart on purpose, because folding any two of them makes one of them a liar: `dropped` (the ring refused it — a loss), `sampled_out` (a declared 1-in-2^k policy skipped it — not a loss), and `suppressed` (a rate policy skipped it — not a loss). `Once` deliberately does **not** count its suppressions, because counting them costs a per-occurrence RMW on a shared line, which is the exact defect this campaign found in the hand-rolled latches; instead the census prints `rate=Once suppressed=UNCOUNTED(by policy)` so that the absence of a count is **itself printed**, and a code whose count genuinely matters declares `OnceCounted` and pays the RMW at its own declaration site.
- **Evidence channels are synchronous.** Measurement lines and validation-layer messages do not travel on the async path (Decisions 9 and 9b).
- The **absence** of records on an armed target is reported as `UNPROVEN`, never as `clean` — and the extension adds three further ways to manufacture that silence, each of which gets its own status (§Decision 17: `UNPROVEN(sampled)`, `UNPROVEN(unsunk)`, `dropped=SATURATED`).

**Performance targets** — every row has a control measured in the same sitting; a number without a control is not a measurement, and this repository has measured its own wall-clock floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol.

| Metric | Target | Control that can go red |
|---|---|---|
| Compile-disabled site | no `emit_impl` symbol reference in the object file | the armed variant of the same fixture **must** show the symbol (§Metrics G1, **at L1-gate — F19**) |
| Runtime-disabled site | ≤ 3 ns | enabled variant of the same site, same bench |
| Enabled, 0 args | ≤ 15 ns median | runtime-disabled, same sitting |
| Enabled, 2×u32 | ≤ 20 ns median | as above |
| Allocations on the producer path, **steady state** | **0**, proven by a **per-thread** counting allocator | armed sink thread must show > 0 on its own thread |
| Allocations on a thread's **first** emit | **≤ 1** (the `thread_local!` destructor registration for the lane guard) and never growing | make `encode` allocate ⇒ steady state > 0 (test 3, F26) |
| Syscalls per record | **0**; one `write` per drain per byte sink | — |
| Producer working set | ≤ 4 cache lines, **unchanged by the extension** | — |
| Sustained rate before drop | ≥ 500 K records·s⁻¹ aggregate, text sink, at default geometry | `sink_sustained_rate` must find the drop knee |
| Sustained rate, **binary sink** | ≥ 5× the text sink in the same sitting | **if it does not separate, L13b is reverted** (G12c) |
| CPU frame-work perturbation, logger idle | not resolvable above the sitting's floor, **on a channel that can respond** | headless schedule bench + interleaved zero control (gate P1, re-specified — F3) |
| Resident memory | `claimed_lanes × 16 KiB` + the fixed table budget (§Decision 3's matrix); `LANES` in `.bss`, gated | section gate G3 |
| Fully-off build | `size_of_val(&LANES) == 0`, no sink thread, no panic hook — **each with a named observation mechanism** | build leg G2 (F4) |
| Session-scale honesty | no counter wraps, no capture silently truncates | G11 (saturation), G12b (rotation), P2 (30-min soak) |

Published reference band: Quill 8-9 ns, NanoLog 7 ns median (both vendor-published, deferred-format); spdlog 242 ns (caller-side format). The ~30× gap **is** caller-side formatting. That single fact organises this design.

---

## Context and constraints

### Affected subsystems
New crate `boyko_log`, depending on `std` only, below everything. `boyko_threadpool`, `boyko_utils`, `boyko_ecs`, `boyko_rhi`, … all depend on it. No cycles. Lane identity is minted **by `boyko_log`**, which is what keeps the threadpool able to log.

### Invariants preserved
1. `clippy.toml`'s `disallowed-types` — no `HashMap`/`HashSet`/`Mutex`/`RwLock`/`Rc`/`RefCell` in this crate at all. **`OUT_LOCK` is NOT a registered hot-path exception, and must not be made one** *(fixes F9)*. Read against the tree: `scripts/check_hotpath_exceptions.py:15-19` requires "the row count per file must match the allow count per file", `:51` counts only `#[allow|expect(clippy::disallowed_types)]` sites, and `:337-341` fails a file whose registry rows exceed its allow count with "registry lists N exception(s) but the file has none left". `OUT_LOCK` is an `AtomicU64`, which the ban does not cover, so `sync_out.rs` carries no `#[allow]` and a row for it would be **drift ⇒ exit 1**. The registry exists for the *type ban*, not for locks in general. What `OUT_LOCK` needs instead is a **protocol that cannot hang** — §Decision 9c — and a mechanical gate over that protocol (G18), not a paragraph in a file that would reject it.
2. **The stdout/stderr machine API — inventoried, not estimated** *(fixes F13, F14)*. Measured this session:
   - `[vk-validation]` is referenced by **31 files**: the producer `crates/boyko_rhi_vulkan/src/debug.rs:114`, `scripts/golden.ps1`, and 29 tests/examples. `golden.ps1:196-202` does **not** do a plain `2>&1` — it routes through `cmd /c … > "$valLog" 2>&1` because PS 5.1 wraps native stderr into `NativeCommandError` records, and `:226` then `Select-String`s the merged file, printing `VALIDATION: clean (0 messages)` in green at zero (`:232`). `crates/boyko_app/tests/vb_bench_query_validation.rs:116-118` pins the prefix **byte-exact including the trailing space** (`"[vk-validation] "`) and its own comment calls it "the gate's entire input".
   - `VB-P1d` is referenced by **16 files** (3 producers under `src/`, 13 consumers).
   - **Correction to v2's M24 claim.** v2 wrote that `vg_occ_split_timing.rs` "parses that merged stream". It does not: `:1115-1117` uses `cmd.output()` and concatenates `out.stdout` **then** `out.stderr` in Rust — two separately buffered pipes, between which interleaving is structurally impossible. The `OUT_LOCK` justification v2 attributed to that consumer was fictional. The real merged-stream consumer is `golden.ps1`, and the real ordering hazard is *within* stdout (F17, §Decision 9).
   - **All of these contracts survive byte-for-byte and remain synchronous** (Decisions 9, 9b). **No v3 feature writes to stdout at all.**
3. Existing `#[should_panic(expected = "boyko-B0002")]` assertions match on a substring, so normalising `error[boyko-B0002]: …` → `boyko-B0002: …` is safe.
4. Codes are never renumbered, never reused. `B9003` is a permanent gap.
5. `#[cold] + #[inline(never)]` on diagnostic helpers (`crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:1-6`).
6. **No new hang class.** `crates/boyko_app/tests/vb_bench_totality_gate.rs:48-49` records that this repository *has no kill-after-timeout pattern to borrow*. Every wait in this design is bounded and, where no acknowledgement is structurally possible, returns immediately with a reason. **v2 violated this invariant with its own `OUT_LOCK`** — an unbounded spin, no release-on-unwind, no re-entrancy story, and a `flush()` timeout path that terminated *in* that unbounded spin (F8). §Decision 9c is the fix, and G18 is its gate.
7. **Principle 0 — claimed as an exception, in writing, with the argument** *(fixes F16)*. `LANES` (2 MiB reserved `.bss`), `CONTROL`, `TARGETS`, `RATE`, `SAMPLE_CTR`, `TARGET_STATS`, `DYN_NAMES`, `SINKS`, `SITE_DICT`, `STAGE` and `SINK_OUT` are durable engine data in plain statics, which Principle 0's named exceptions do not obviously cover. The exception is claimed on **dependency inversion**: `boyko_log` sits *below* `boyko_ecs` — every crate including `boyko_ecs` depends on it — so there is no `World` it could store into without a cycle, and there is provably no `World` in existence at the three moments the logger matters most (before `boot()`, inside a driver callback, inside a panic hook). The rule's substance is honoured where it can be: everything the ECS *can* own — `LogRing`, `LogStats`, `LogCensus` — is a `Resource` on `VmReservation`-backed engine storage (`VmColumn`), never a `Box<[u8]>` or a `Vec`. The cost of the exception is stated rather than hidden: no `Query`, no change detection and no `EnableTag` over `CONTROL`; the mitigation is `CONTROL_EPOCH`, an `O(1)` repaint signal a UI polls instead of subscribing (§Decision 23).

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
**Why.** The `&&` short-circuit is what guarantees arguments are never evaluated — `log`'s exact shape, and why its docs warn against side-effecting arguments. Unreal explicitly does *not* give this guarantee and documents `UE_LOG_ACTIVE` as the workaround. The per-target compile ceiling is Unreal's two-verbosity model, which `log`/`tracing` lack. The runtime gate is one `Relaxed` load from `static CONTROL: [AtomicU8; 256]` plus one `and` — puffin's measured ~1 ns `AtomicBool` shape, generalised (Decision 14).

**Alternatives rejected.** *Cargo features for the ceiling* — features are additive and unified; one crate enabling `max_level_trace` re-enables it for everyone. `option_env!` in a `const fn` has no such failure mode; staleness is closed by `build.rs` emitting `cargo:rerun-if-env-changed=BOYKO_LOG_MAX_LEVEL`. **`GLOBAL_CEILING` is a `const` item inside `boyko_log`, referenced as `$crate::GLOBAL_CEILING`; the `option_env!` is never expanded into a caller crate**, where no rerun directive exists (N27).

**Trade-off.** Changing the ceiling rebuilds the workspace. `MAX_TARGETS = 256` is a hard cap.

### Decision 3: Statics in `.bss`; `Off == 0`; a genuinely-off build *(extends v1, fixes M21; re-specified by F4, F21, F25)*
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

**Gated, not assumed** (M21): `.bss` residency of a `MaybeUninit` static on PE/COFF is a toolchain behaviour, so gate **G3** runs `llvm-readobj --sections` (or `objdump -h`) over the test binary and asserts the section owning `LANES` carries a size with no raw data.

**G2 is re-specified: each of its three legs names its observation mechanism** *(fixes F4)*. v2's G2 asserted "`size_of_val(&LANES) == 0` and that no sink thread is spawned" and Decision 3 added "no panic hook", with no mechanism for either non-size leg — so `boot()` could spawn the thread and install the hook while `LANE_ARRAY_LEN` was 0 and G2 would still be green.

| Leg | Mechanism | Named red state |
|---|---|---|
| (a) size | `const _: () = assert!(LANE_ARRAY_LEN == 0)` in the off leg. **This is a const tautology and is kept only as env-plumbing proof** — it reds when `BOYKO_LOG_MAX_LEVEL=off` fails to reach the crate, which is a real failure this repository has seen in other guises, and it is worth exactly that much. It is annotated as such in the test so nobody mistakes it for the claim | unset the CI leg's env var |
| (b) no sink thread | **OS thread count across `boot()`**: Windows `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)` counting threads whose `th32OwnerProcessID == GetCurrentProcessId()`; Linux `std::fs::read_dir("/proc/self/task").count()`. Test-only, ~15 lines, one `#[cfg]` pair. **Its own control**: the same fixture spawns one deliberate `std::thread` and asserts the probe's count rises by exactly 1 — so a probe that always returns a constant reds before it can certify anything | make `boot()` spawn unconditionally ⇒ count rises ⇒ leg (b) reds |
| (c) no panic hook | **Behavioural, not identity-based** — `std::panic::take_hook()` is destructive and returns an unidentifiable `Box<dyn Fn>`. The fixture installs its **own** probe hook before `boot()`, then panics under `catch_unwind`, and asserts (i) the probe fired **exactly once** and (ii) the captured stderr contains **no** `boyko-log` marker line | make `boot()` chain its hook unconditionally ⇒ the marker appears ⇒ leg (c) reds |

**What G2 cannot claim**: that the off build has *no* cost. It has one: the crate is still compiled, linked and depended upon by every other crate. G2 bounds the *runtime* footprint to zero lanes, zero threads and zero hooks; it says nothing about compile time or binary size, and the `shipping` profile (Decision 25), not `off`, is the configuration a real title ships.

**`.bss` budget matrix, stated in full** *(fixes F25 — v2 left `STAGE_BYTES`'s backing store unspecified, and "no `Vec`/`Box` in any signature" is carefully narrower than the Principle-1 claim a reader takes from it)*. Every one of these is a `.bss` static, demand-zero, never heap:

| Table | `dev` | `shipping` | Note |
|---|---|---|---|
| `LANES` | 128 × 16 KiB = 2 MiB | 32 × 16 KiB = 512 KiB | reserved; resident is `claimed_lanes × 16 KiB` |
| `RATE` | 512 × 64 B = 32 KiB | same | per-code; `Once` no longer uses it (§Decision 8) |
| `SAMPLE_CTR` | `MAX_LANES` × 256 × 2 B = 64 KiB | 16 KiB | one row per lane, producer-private |
| `TARGET_STATS` | 256 × 64 B = 16 KiB | same | consumer-written |
| `CONTROL` + `TARGETS` + `DYN_NAMES` | 256 B + 2 KiB + 2 KiB | same | |
| `STAGE` | 256 KiB | 256 KiB | **`static STAGE: UnsafeCell<[u8; STAGE_BYTES]>`** — sink-owned, reused every drain, never allocated |
| `SITE_DICT` + `SINK_OUT` | 64 KiB + 1 MiB | 64 KiB + 256 KiB | binary sink only; absent unless `BinarySink` is configured |
| **Total reserved** | **≈ 3.4 MiB** | **≈ 1.1 MiB** | resident is a small fraction of each |

**Honest floor when "on"**: the matrix above, one OS thread, one process-global panic hook, one `VmReservation`-backed `LogRing` when the ECS seam is enabled, and a mandatory dependency edge from every crate. That is the cost of the system existing; it is stated, not smoothed.

**Off-build dead code** *(fixes F21)*: v2 set `LANE_ARRAY_LEN = 0` while §Algorithms B scanned `start..start+MAX_LANES` and indexed `LANES[i]` — dead but panicking code that G2 could not distinguish. The claim scan is therefore written over `LANES.iter().enumerate()` with a `spread` rotation, so an empty array is zero iterations and the function returns "exhausted" immediately; and in the off build no call site survives the const gate anyway, including the `Warn`/`Error` fallback, because `GLOBAL_CEILING == Off` deletes every level.

### Decision 4: SPSC byte ring per lane; claim-on-first-use; consumer-only reclaim
**What.** Each lane is a single-producer/single-consumer byte ring. A thread claims a lane by `load`-then-CAS scan on `owner` (`FREE → token`) on first emit; a TLS guard's `Drop` stores `RETIRING`; the sink stores `FREE` once it observes `RETIRING && read == write`.

**Why.**
- True SPSC is why the threadpool's lane router is not reused: it maps every non-pool thread to lane 0, which would put the window thread, the present thread and every test-harness thread on one ring.
- The retire protocol closes the thread-exit hazard the research names as the most common lock-free-logger bug. It is trivially sound here: the ring is a `static` that never moves and records are POD with no `Drop`, so a retired-undrained lane leaks nothing.
- The producer caches the opposite cursor. The one published measurement on this question found padding **alone** made a ring *slower* — both threads still read the opposite cursor every operation — and only opposite-cursor caching *plus* padding moved throughput from ~32 to ~440 M ops·s⁻¹. We do both and treat padding as a hypothesis with an ablation bench, matching this repo's own `reference-componentpool-cache-stagger` lesson.

**Claim scan is `load`-then-CAS** (M10): `if owner.load(Relaxed) == FREE { try CAS }`. An unconditional `compare_exchange` over 128 lanes takes every occupied lane's line exclusive and invalidates up to 127 producers — the exact defect this repo already fixed at `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs:36-51` ("load first, store once"). The scan additionally starts at `hash(thread_id) % MAX_LANES` so concurrent claimants do not convoy on lane 0.

**Alternatives rejected.** *Double-buffer + wholesale swap* (`EventBuffer::swap_and_flatten`) — needs a quiescence point that boot code, the present thread and a driver callback do not have. *One MPMC ring* — CAS on every push, reintroducing the contention the per-lane design removes by construction.

### Decision 5: Overflow drops, counts, and reports — corrected arithmetic, saturating counters, one aggregated report *(fixes F6, F24; extended by X4)*

**The defect first, because it is the reason this decision was rewritten.** v2's admission control read:

```
limit = LANE_BYTES - if level == Error { 0 } else { ERROR_RESERVE }   // 14336 for non-Error
free  = limit - (w - read_cached)                                     // u32 subtraction
if free < pad + need { … }
```

`ERROR_RESERVE = LANE_BYTES/8 = 2 KiB`. The Error reserve exists **precisely to be exercised** in the state where the used span exceeds `LANE_BYTES - ERROR_RESERVE` — v2's own test 5 constructs that state. There, a subsequent non-`Error` emit computes `14336 - 15000` in `u32` = `4_294_966_632`, the guard `free < pad + need` is **false**, and the producer writes at `off = w & MASK` over bytes the consumer has not staged. The consumer then walks a torn header and calls `decode` through a corrupted `*const LogSite`. That is the same UAF class B1 was folded to remove, re-entered through the arithmetic instead of through the ordering, on the designed-and-tested path.

**What (corrected).** Never block. Admission control is formulated so that **no unsigned subtraction can go negative**:

```
CAPACITY = LANE_BYTES - 1                    // one slot reserved: distinguishes full from empty
used     = w.wrapping_sub(read_cached)       // INVARIANT: used <= CAPACITY   (induction below)
avail    = CAPACITY - used                   // cannot underflow, by the invariant
budget   = if level == Error { avail } else { avail.saturating_sub(ERROR_RESERVE) }
if budget < pad + need { refresh read_cached (Acquire); recompute; if still short => drop }
```

**Why this form is sound and the old one was not.** The invariant `used ≤ CAPACITY` is *inductive over the producer's own admissions*: `read_cached ≤ read ≤ w` always (the consumer only advances `read` toward `w`, and `read_cached` is a stale copy of `read`), and the producer only ever publishes `w' = w + pad + need` after proving `pad + need ≤ avail = CAPACITY - used`, hence `used' ≤ CAPACITY`. The base case is `used = 0`. The reserve is now applied as a **`saturating_sub` from the available space**, not as a subtraction from the capacity, so the "reserve already eaten" state yields `budget = 0` — a refusal — instead of a 4-billion-byte licence. `saturating_sub` on `u32` lowers to `sub` + `cmov`: still branchless, one extra instruction on a path that already has a compare.

**Counters saturate** *(X4)*. `dropped` / `dropped_bytes` are `AtomicU32` and **saturate at `u32::MAX`** rather than wrapping: one `load` guard on the already-cold drop path. With no drain running — post-`shutdown()`, `SinkMode::Manual`, a dead sink — a wrapping counter reaches `u32::MAX` in ~65 s at 66 M offers·s⁻¹ and then reports a small, credible, **wrong** number. That is a silently truncated capture in a new costume, which is exactly the pattern this campaign exists to kill. `AtomicU64` was rejected: an 8-byte RMW is more expensive and *still* would not make the reported value unambiguous, because the reader cannot tell "4 billion" from "the ceiling". The census renders the ceiling as `dropped=SATURATED(>=4294967295)` — a token that cannot be mistaken for a count. Gate **G11** is two-sided over exactly this.

**One aggregated drop report per drain, not one per lane** *(fixes F24)*. v2 emitted a synthetic `boyko-W0102` "per drain per lane": at 125 Hz × 128 lanes that is ~16 000 sink-generated records·s⁻¹ against a stated ~500 K·s⁻¹ formatting budget — 3 % of the budget spent by the drop reporter competing with the drops it reports, and unbudgeted. v3 emits **one** `W0102` per drain carrying `lanes_affected`, `records`, `bytes` and a `SATURATED` flag: 125 records·s⁻¹, a fixed cost. Per-lane detail lives in the census, which is polled, not streamed. Clearing stays `fetch_sub(observed)` — never `store` — because a producer may increment concurrently; a saturated counter is cleared by a CAS to 0 that fails harmlessly if a producer re-saturates.

**Lane-exhaustion fallback** (M26, unchanged): a thread that cannot claim a lane does **not** silently drop `Warn`/`Error`. It falls back to the synchronous channel (`write_oracle_line`, now bounded — §Decision 9c) for those two levels only, and counts `Info`/`Debug`/`Trace` into `UNLANED_DROPPED`. The cost is paid only in the exhausted case, and a test harness that exhausts lanes therefore cannot lose a severe record.

**Why.** Blocking on `error!` inside a driver callback under a storm is a deadlock. Silent loss turns a logger into a source of false confidence — the exact class this campaign exists to kill.

**Alternatives rejected.** *Block-on-full* (spdlog's default) — a mutex by another name. *Overrun-oldest* — destroys the record that reported the cause in favour of the one that reported the consequence.

### Decision 6: One diagnostic-code registry, kept honest by eight mechanical checks over a SPECIFIED walker *(fixes F5, F20)*
**What.** `crates/boyko_log/src/codes.rs` holds a single `codes! { … }` invocation generating: a `pub const` per code, a **dense** `static DIAGNOSTICS: [DiagInfo; N]` sorted by number, a dense `code_idx` per code (the `RATE` index — M12), and `explain()`. A literal `"boyko-…"` outside the registry is a build failure. Class is a **type** property: `warn!` takes `WarnCode`, `error!` takes `ErrorCode`, `PanicCode` is distinct — a class mismatch does not compile.

**The checks** live in `crates/boyko_log/tests/code_registry.rs` — an **integration** test, because `cargo test --workspace --lib` does not build `tests/`, a blind spot that cost this repo four commits.

#### The walker: ONE pass, THREE streams, and every check names the streams it consumes *(fixes F5)*

v2 specified no walker, and its checks 3 and 6 then required **opposite** behaviour from the one it did not specify. Measured in this tree:

- `crates/boyko_ecs/src/ecs/core/app/app.rs` contains **24** occurrences of `boyko-B1802`; **18** are inside `/// # Panics` doc comments (`:267`, `:283`, `:303`, `:333`, `:365`, …), **1** is the panic message string (`:867`), and **5** are `#[should_panic(expected = …)]` inside the in-`src` `#[cfg(test)]` module (`:898`-`:939`). *(The review said 28 doc-comment sites; the measured number is 18. The finding stands — the count does not.)*
- `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:302,304,309,314,317,318` mentions `boyko-B9001` / `B9002` / `B9004` / `B9101` in doc comments, plus a `boyko-B900x` **pattern** at `:302` that is not a code at all.
- `crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:46` — `/// Diagnostic code: \`boyko-B0002\`.`

So: a substring scan that includes comments makes check 3 (no orphans) satisfiable by writing a `# Panics` line — v1's vacuity relocated from `.md` into `.rs` — while a scan that excludes comments makes check 6 (panic-class placement) red on those same 18 sites the day L2 lands. Both cannot hold under one unspecified walker.

**The walker is specified.** One pass over each `.rs` file produces three disjoint streams, and it is the **same walker** that backs `print_census.rs`, so its `#[cfg(test)]`-region rule and its `src/bin/` exclusion are written once and exercised by two tests:

| Stream | Contents | Consumed by |
|---|---|---|
| **CODE** | source text with `//`/`///`/`//!` line comments, `/* */` block comments, string/char literals and `#[cfg(test)]` regions removed | checks 3, 3b, 6, 7 and the print census |
| **LIT** | the contents of string and char literals only | check 4 |
| **TEXT** | the whole unstripped file, plus all of `docs/**.md` | checks 0, 4 |

Check 3 matches a **standalone identifier token** (`B1802`, or the path form `codes::B1802`) in CODE — never a substring of `boyko-B1802`, which after stripping does not exist in CODE at all. Check 4 matches the `boyko-[BEW]\d{4}` **literal** in LIT ∪ TEXT, so a doc comment or a book page naming an unregistered code is still caught. Check 6 runs over CODE, so doc mentions and the in-`src` `#[cfg(test)]` `should_panic` strings are invisible to it. The walker is deliberately not a Rust parser; its failure mode (a `#[cfg(test)]` block over-reaching to end-of-file) is the same one `scripts/check_hotpath_exceptions.py:164-201` already documents and accepts for the same reason.

#### `Live` vs `Pending`: how L2 commits alone on a grandfathered corpus *(fixes F20)*

L2 seeds the registry with the 9 existing codes, but the *identifiers* `codes::B9001`, `codes::W1501`, … do not appear in CODE until L6/L7/L8 migrate the emitters. A check-3 that scans identifiers therefore reds at L2, and one that scans literals is vacuous. Each registry row carries a status:

- **`Live`** — check 3 requires ≥1 CODE occurrence. Register a `Live` code and emit it nowhere ⇒ **red**.
- **`Pending(rung)`** — check **3b** requires **zero** CODE occurrences. Emit a `Pending` code ⇒ **red**, which forces the row to be flipped to `Live` in the same commit that lands the emitter. A `Pending` row cannot rot silently, because the day it acquires an emitter it reds.
- Check **3c**, armed at L8c only: `Pending` count == 0. This is the migration's real exit criterion, and it is one integer.

#### The eight checks

Numbered **0 through 7**. Checks `3b` and `3c` are legs of check 3, not additional checks — the count is eight, and `codes_tidy!` generates all eight for a downstream table.

| # | Check | Stream / corpus | Red state that must be demonstrated once |
|---|---|---|---|
| 0 | **Corpus is non-empty**: `files_scanned ≥ 500`, and the pinned sentinel `boyko-W1501` is found | TEXT | point the walker at a wrong root → red |
| 1 | Numbers strictly increasing ⇒ no duplicates (also a `const _: () = assert!`) | registry | add a duplicate |
| 2 | `docs/diagnostics/<code>.md` exists, non-empty, has `## What happened` / `## Why` / `## How to fix` | `docs/diagnostics/` | delete a section heading |
| 3 | **No orphans**: every `Live` code's identifier appears ≥1× as a standalone token | CODE, excluding `codes.rs` | register a `Live` code, emit it nowhere |
| 3b | **No premature emitters**: every `Pending` code's identifier appears **0×** | CODE | emit a `Pending` code without flipping its row |
| 3c | **Migration complete** (armed at L8c): `Pending` count == 0 | registry | leave one row `Pending` |
| 4 | **No undeclared**: every `boyko-[BEW]\d{4}` literal resolves to a registry entry | LIT ∪ TEXT (incl. `.md`) | write `boyko-W9003` in a doc |
| 5 | Every `Live` `W`/`E` code is observed by ≥1 test, with `tests/untested_codes.txt` (a **data file**, excluded from its own scan) checked **in both directions** | `crates/**/tests/**`, `#[should_panic(expected=` | allowlist a code that has a test |
| 6 | Panic-class `B` codes appear only inside a `#[cold] fn … -> !` or a `panic!` | CODE | emit a `B` code from a `warn!` |
| 7 | Every `LogTarget` impl in the workspace resolves to a `targets!` row or a `define_target!` expansion | CODE | hand-write a `LogTarget` impl |

**Why the corpus rules changed.** v1's check #3 was vacuous because check #2 *mandates* a doc file naming the code and v1's scan included `.md`. v2 narrowed the corpus to `.rs` and reintroduced the same vacuity through comments. v1's check #5 was self-defeating: the allowlist named identifiers and lived inside the file being scanned. Check #0 closes the third failure in the same family — a walker that resolves its root badly scans zero files and reports zero orphans, green. rustc's tidy pins a sentinel for exactly this reason.

**What these checks CANNOT claim.** They are engine-scope. They prove nothing about a game's or a mod's registry — which is why `codes_tidy!` (Decision 19) generates the same eight checks over a caller-supplied root and prefix, and why G9's assertion message says so in the failure text rather than in this document.

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

### Decision 8: Rate policy is DECLARED on the code and APPLIED per site; `Once` is a site-local latch that degrades to a pure load *(fixes M11, M12, F11; extended by X3)*

**What.** Each `W`/`E` code declares `Every` / `Once` / `OnceCounted` / `EveryN(n)` / `MinIntervalMs(ms)` in the registry. The *mechanism* differs by policy, and that distinction is the fix for F11:

| Policy | State lives in | Scope | Steady-state cost |
|---|---|---|---|
| `Once` | a macro-generated `static FIRED: AtomicBool` **beside the call site's `LogSite`** | **per SITE** | one `Relaxed` load from a site-private line, not-taken branch |
| `OnceCounted` | the same site-local static + an `AtomicU32` | per SITE | one load; **one RMW per suppressed occurrence** — opt-in, cost stated at the declaration |
| `EveryN(n)`, `MinIntervalMs` | `static RATE: [RateSlot; MAX_CODES]`, dense `code_idx` | per CODE | one RMW per occurrence |
| `Every` | — | — | nothing |

**Why `Once` had to become per-site** *(F11)*. `RATE` is indexed by `code_idx`, so a code-scoped `Once` fires **once per code, not once per site**. Read against the tree: the migration routes `crates/boyko_rhi_vulkan/src/device.rs:3100`, `:3158` and `:3189` — three independent capability degradations — through **one** code `W2102` with `RatePolicy::Once`. A device lacking all three would report **one** and silently lose two, uncounted (and `Once` deliberately does not count). That defeats the migration's own stated purpose, which is `crates/boyko_app/src/host.rs:228-233`'s written argument that a RELEASE-build degrade-to-off must be observable. `W2202` has the same shape across `bindless.rs` and `mesh_geometry_table.rs`.

The resolution is not to mint three codes — a code names a *class of condition*, and three near-identical codes make `explain()` worthless — but to put the latch where the diagnostic value is: **the site**. The macro expands, next to the `&'static LogSite` it already emits, a `static FIRED: AtomicBool = AtomicBool::new(false)`. Cost: 1 byte of `.bss` per `Once` site. Benefit beyond correctness: the steady-state load hits a **site-private, never-contended** line instead of a shared `RATE` line, so this is strictly cheaper than v2 on both axes.

```
Once:  if FIRED.load(Relaxed) { return; }          // steady state: load only, private line
       if !FIRED.swap(true, Relaxed) { emit }      // exactly one RMW, ever, per site
```

**Why the no-store property matters.** The audit found five hand-rolled latches with two implementations, one wrong: `crates/boyko_render/src/render_path_config.rs:311-313` and `:335-337` execute `swap(true, Relaxed)` on a shared line **inside an `#[inline]` per-frame reader, every frame forever** once the divergence holds. v1's replacement kept a per-frame `suppressed.fetch_add` — the same defect wearing a policy name.

**Suppression is reported as policy, and the absence of a count is itself printed** *(fixes F10)*. The review is right that v2 contradicted its own Goal: "loss is counted and reported; never silent" against "the `Once` count … is **not reported**", with the census reporting neither (`suppressed` is neither `records` nor `dropped`). v3 resolves it in three moves rather than by softening a sentence:

1. The Goal now says suppression is **not loss** and is reported **as policy** (§Goal, functional bullet 3).
2. The census prints, per `Once` code, `rate=Once fired=1 suppressed=UNCOUNTED(by policy)`. A reader can never mistake a missing count for an absent phenomenon, which is the property that actually matters.
3. A code whose suppressed count genuinely matters declares **`OnceCounted`** and pays one RMW per suppressed occurrence — at its own declaration site, visible in the registry, with the cost written in the row. The engine's own `W2102`/`W2202` use plain `Once`; a game is free to choose otherwise.

**`EveryN(n)` requires `n` to be a power of two** *(X3)*, enforced by `const _: () = assert!(n.is_power_of_two())` inside `codes!`, so the test is `count & (n-1)` instead of `count % n`. v2's arbitrary `n` mis-samples across the `u32` counter wrap (~12 h at 100 K·s⁻¹) — invisible in a 300-frame bench, wrong in a session. The fix is *also* cheaper: an `and` for a division. Strictly better on both axes.

**Layout.** `RateSlot` is 64 B, one per cache line — four unrelated codes sharing a line (v1's 16 B slot) is false sharing between subsystems that have nothing to do with each other. `MAX_CODES = 512` ⇒ 32 KiB, in the same `.bss` regime as `LANES`.

### Decision 9: Measurement output does NOT go through the logger — and it goes through stdout's OWN buffer *(amended by F17)*
`report!` is a separate, explicitly **synchronous** macro: it formats into a **caller-owned stack buffer first**, then takes `OUT_LOCK`, then does one `write_all` **through `std::io::stdout()`'s handle** (i.e. through the same `LineWriter` every surviving `println!` uses), then flushes, then releases. It carries `VB-P1d` / `VB-P4` / `VB-SV0-S1.5` / the R0 table and nothing else.

**Why the buffer, not a raw handle** *(fixes F17)*. v2 said "one `write_all` to stdout, **unbuffered**". Bypassing `stdout()`'s `LineWriter` does not make ordering stronger — it makes it *undefined against every other stdout writer*. Through L8b and permanently thereafter, the 58 `boyko_shaderdsl` CLI prints, the 23 in-`src` test prints, libtest's own output and every not-yet-migrated `println!` all sit in that `LineWriter`. `OUT_LOCK` is process-internal and knows nothing about it, so a raw-handle `report!` can emit *between* a `println!`'s buffered bytes and their flush. Writing through the same handle costs one memcpy into a buffer and one flush syscall on a cold path, and buys the ordering the byte-frozen contract actually depends on. It also keeps `golden.ps1:196-202`'s `cmd /c` wrapper — which exists because PS 5.1 mangles *native* stderr — seeing exactly the bytes it sees today.

**Why formatting happens before the lock**: a `Display` that panics inside the critical section would otherwise leak the lock permanently and hang the panic hook's flush (F8). Formatting first makes an unwinding argument impossible to reach while the lock is held.

**Why a separate channel at all.** Those lines are a machine API: `VB-P1d` alone is referenced by 16 files, and `crates/boyko_app/src/runner.rs:3108-3109` documents the line as byte-identical *because* `vg_occ_split_timing.rs` parses it. *(v2 cited `:3084-3085`, which are `.zip(cull_dispatch_ns)` / `.zip(shade_ns)` and document nothing — corrected.)* Routing them through an asynchronous, timestamp-merged, droppable channel would reorder them against libtest's output, make them droppable under load, and lose them on a crash. A benchmark whose output can vanish is not a benchmark. **`report!` is explicitly NOT a game-facing API**: it formats on the caller and serialises the frame.

### Decision 9b: The validation messenger is NOT TOUCHED AT ALL — v1's migration stays withdrawn, and v2's "harmless" edit is withdrawn too *(fixes B8 and F12)*
**What.** `crates/boyko_rhi_vulkan/src/debug.rs`'s callback keeps its `eprintln!("[vk-validation] {}", msg.to_string_lossy())` at `:114`, byte for byte, on `stderr()`'s own lock. Nothing about it changes. It is added to `tests/print_allowlist.txt` with the reason "byte-frozen gate-oracle channel; see Decision 9b" — and because that allowlist is checked **in both directions**, a future removal of the site reds the tidy test rather than silently orphaning the entry.

**Why v2's edit is withdrawn** *(F12)*. v2 justified touching the site by "removal of the per-message `to_string_lossy()` allocation". That justification is largely false and the edit is actively harmful:
- `CStr::to_string_lossy()` returns `Cow::Borrowed` for valid UTF-8 — **no allocation on the normal path**. It allocates only for invalid UTF-8, which is not the path any gate runs.
- Writing "the `CStr` bytes directly" **changes the emitted bytes** exactly in the invalid-UTF-8 case (today `U+FFFD`, after: raw bytes) — on a channel this document declares byte-frozen and gate-oracle, pinned byte-exact including the trailing space at `crates/boyko_app/tests/vb_bench_query_validation.rs:116-118`.
- `eprintln!` currently takes `stderr()`'s own lock. Moving the site to `OUT_LOCK` would let the ~90 surviving `eprintln!` sites interleave *inside* a `[vk-validation]` line — a regression **introduced by M24's own fold**.

Trading a non-allocation for a byte change and an interleaving hazard, on the one channel whose value is that it has not moved, is a bad trade. The site stays.

**Why the channel stays synchronous.** Verified this session at `scripts/golden.ps1:201,226,232`: the scan runs over the child's *merged* stdout+stderr file and prints `VALIDATION: clean (0 messages)` in green at zero. Today the message is on the wire before `vkQueueSubmit` returns. Behind a 16 KiB lane drained ≤ 8 ms later, three loss modes are all reachable *in exactly the runs the gate exists for*: a storm overflows the lane (a storm is what an error looks like); an error preceding a driver abort loses everything undrained; a rate policy suppresses. Each yields green. **Decision 9's own rule — a gate whose evidence can vanish is worse than no gate — applies here verbatim, and v1 violated it.**

**The E12 conflict is resolved, not finessed.** "No lock, no syscall" is a rule about **frame-hot paths**. A validation callback under an enabled validation layer is not one: validation is off by default, and when on, the run is already an order of magnitude slower. With the site untouched, the conflict does not even arise — the lock in question is `stderr()`'s, which predates this plan.

**What is *added*:** `boyko-E2101` (below) and the `LOG-CENSUS`, both of which make *absence* loud rather than making presence prettier. Neither writes to the messenger's channel.

### Decision 9c: `OUT_LOCK` — bounded acquire, re-entrancy-aware, unwind-safe, and it steals rather than hangs *(fixes F8)*

v2 specified an `AtomicBool` spin lock with **no bound, no release-on-unwind and no re-entrancy story**, taken by `report!`, `write_oracle_line`, the console sink and (as of v3, additionally) sync-routed targets and `SINK_REQ` writes. Three concrete hangs followed, each on the error-of-the-error path — the one place a logger must not fail:

- **E14** ("panic inside a sink — the sink catches, **direct-writes**, continues"): if the panic happened while the sink held `OUT_LOCK`, the direct-write is a non-reentrant self-deadlock.
- **`flush()`'s timeout** writes `boyko-E0105` via `write_oracle_line` (§Algorithms D step 5) — a *bounded* wait terminating in an *unbounded* one.
- A `Display` panicking inside `report!`'s format leaked the lock permanently; the panic hook's flush then hung the process.

Against an invariant the same document states as "no new hang class", citing `vb_bench_totality_gate.rs:48-49`. The protocol is therefore specified, not assumed:

```rust
static OUT_OWNER: AtomicU64 = AtomicU64::new(0);   // 0 = free; else an opaque thread token
static OUT_STEALS: AtomicU32 = AtomicU32::new(0);
static OUT_REENTRANT: AtomicU32 = AtomicU32::new(0);

/// RAII. `Drop` releases on the normal path AND on unwind.
struct OutGuard { mode: OutMode }   // Held | Reentrant | Stolen
```

1. **Format before you lock.** Every caller renders into a caller-owned stack buffer first (`report!`, `write_oracle_line`, the console sink). No user `Display` and no `core::fmt` runs inside the critical section, so an unwind cannot originate there.
2. **Re-entrancy is detected, not deadlocked.** Acquire is `CAS(0 → my_token)`. On failure, if `OUT_OWNER == my_token` the caller is re-entrant (the E14 case): the guard is `Reentrant`, the bytes are written **prefixed by a newline** so they cannot corrupt the *start* of the outer line, and `OUT_REENTRANT` increments. The census reports it.
3. **Acquire is bounded.** Spin with `spin_loop()` backoff, then `yield_now()`, to a **50 ms** deadline. On expiry the writer **steals**: it writes anyway, increments `OUT_STEALS`, and emits `boyko-W0110` once. An interleaved line is a legible defect; a hung process is not. This is the explicit trade, and it is the only shape compatible with Invariant 6.
4. **Release is unwind-safe** by construction — `Drop` on `OutGuard`, and the guard is the only way to obtain write access.
5. **The panic hook and `flush()`'s timeout path use the same bounded acquire**, so no bounded wait terminates in an unbounded one.

**Gate G18** (L3-gate), two-sided: (a) a thread that acquires `OUT_LOCK` and then panics releases it — a second thread's `report!` completes within the deadline; (b) a re-entrant `report!` from inside a sink panic handler **completes** and increments `OUT_REENTRANT`, instead of deadlocking. **Red state**: replace the guard with a bare `store(false)` after the write ⇒ (a) hangs and the test's own deadline reds it. **What G18 cannot claim**: that the output is never interleaved. Under a steal it *is*, deliberately; `OUT_STEALS > 0` in the census is the honest report of that, and a nonzero value in a golden run is itself a defect signal.

**`OUT_LOCK` gets no row in `docs/HOT-PATH-EXCEPTIONS.md`** — see Invariant 1 for why that would red CI. Its justification lives in `sync_out.rs`'s module doc and here.

### Decision 10: One dedicated sink thread; adaptive park; `Manual` mode for hermetic tests
**What.** Default `SinkMode::Thread`: one thread, started at `boot()`, draining all lanes, staging, sorting by `tsc`, formatting once, fanning out. Park policy is **adaptive**: `park_timeout(0)` — immediate re-drain — while any lane yielded records last pass; `park_timeout(8 ms)` when every lane was empty. Producers **never** unpark (that is a syscall per record); `flush()` and `shutdown()` do. `SinkMode::Manual` requires an explicit `drain()`; it exists for hermetic tests, CLI tools and the zero-alloc gate, and for nothing else.

**Throughput, stated rather than deferred** (M19). At the default geometry, a lane holds `16 KiB / ~40 B` ≈ 400 records; with immediate re-drain under load, per-lane sustained capacity is bounded by the sink's formatting rate, not by the park interval. Design number: **≥ 500 K records·s⁻¹ aggregate** with a `core::fmt` cost of ~1-2 µs per formatted record on one thread. Consequence, stated plainly: **a `trace!` inside a per-entity loop is lossy by construction** — at 15 ns/record a single producer can offer 66 M records·s⁻¹ against a consumer two orders of magnitude slower. Gate **`sink_sustained_rate`** at L3 measures the knee and must show a nonzero drop count above it; the plan is not allowed to ship with the knee unmeasured.

**Alternatives rejected.** *Drain from an ECS system at `Last`* — makes the consumer MPSC when combined with any other driver, and ties log liveness to a running schedule, so boot and shutdown diagnostics vanish. *Drain from the frame loop* — a syscall in the frame.

### Decision 11: `rdtsc` on x86_64 with a boot-time invariance probe *(citation corrected)*
The tree's own note on the QPC-backed `Instant::now()` is at `crates/bench_bevy_vs_boyko/benches/profile_spawn.rs:229-231`: "each **pair** of `now()` calls costs **~20-30 ns**", with the criterion bench that would measure it defined at `:233-241`. **v2 claimed "~25 ns/call and ~60 ns/pair — *measured*", which is 2× the cited source and attributes a measurement to a prose comment.** Corrected: the tree records ~20-30 ns per *pair*, i.e. ≥ 10 ns per call, and that number is a comment, not a recorded run. The argument survives the correction with room to spare — a ≥ 10 ns clock inside a 15 ns whole-record budget is not a design — and the plan does not claim more precision than the source carries. Tracy uses `rdtsc` guarded by an invariant-TSC check. AVX2 baseline ⇒ Haswell+ ⇒ invariant TSC since Nehalem.

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

**Replacement — with the expansion form stated, because the naive one does not borrow-check** *(fixes F22)*. v2 described `dsp!` as "yielding `&str` from a caller-owned stack buffer" without saying which form. The block form `{ let mut buf = […]; &buf }` returns a reference to a local and does not compile. The form that works is a **by-value temporary in argument position**, whose `as_str()` borrow is extended to the end of the enclosing statement by Rust's temporary-lifetime rules for function-call arguments:

```rust
/// Renders `Display` into a stack buffer OWNED BY THE ARGUMENT EXPRESSION and
/// yields `&str`. Expands in ARGUMENT POSITION, so it runs to completion
/// BEFORE `emit_impl` is called and before any lane state is touched.
/// Overflow truncates and sets STR_TRUNCATED; it can never overrun a ring.
pub struct DspBuf<const N: usize> { buf: [u8; N], len: u16, truncated: bool }
impl<const N: usize> DspBuf<N> {
    #[inline] pub fn render(v: &impl core::fmt::Display) -> Self { /* fmt::Write into buf */ }
    #[inline] pub fn as_str(&self) -> &str { /* UTF-8 safe by construction of the writer */ }
}
#[macro_export] macro_rules! dsp {
    ($e:expr)             => { $crate::DspBuf::<256>::render(&$e).as_str() };
    ($e:expr, $n:literal) => { $crate::DspBuf::<$n>::render(&$e).as_str() };
}
// warn!(Render, codes::W2201, "material {} rejected", dsp!(mat_id));
```

Rust evaluates arguments before the call, and the `DspBuf` temporary created inside the argument expression lives to the end of the full statement, so the `&str` is valid for the whole emit. **Nothing between lane acquisition and the `Release` store can call user code**, because encoding operates on already-materialised POD and `&str`. Backed by `debug_assert!(!IN_EMIT.replace(true))` in `emit_impl` — the guard exists to catch a future violation, not because one is representable today.

**Trade-off, both halves stated.** A `Display` costs `core::fmt` at the call site — the honest price, legible in the source, which is precisely why v1's invisible version was worse. And the `DspBuf<256>` is constructed **by value**: worst case a 264-byte stack object the compiler is free to (and in practice does) construct in place. `dsp!(x, 32)` exists for the sites where 256 bytes of stack in a deep call is not acceptable, and the truncation is reported (`STR_TRUNCATED`) rather than silent.

### Decision 14: ONE byte per target owns level + sampling + sync routing; `LogFilter` is deleted *(fixes M14; extended by X1)*
v1 had `LogFilter { ceilings: [Level; 256], dirty: bool }` mirroring `CEILINGS`, synced by a hand-rolled flag — two sources of truth for one datum, with a public `set_target_level()` writing only one of them, so the next unrelated `dirty` flip would silently push a stale value over the live one. It also re-offended the "capability/state is not a bare bool" rule.

**Replacement.** `CEILINGS` is renamed **`CONTROL`** and its byte is packed:

```
bit  [0..2]  level      (Off | Error | Warn | Info | Debug | Trace)
bits [3..6]  sample shift k  (0 = every record; else deliver 1 in 2^k)   — Decision 20
bit  [7]     sync route      (format on the caller, write synchronously) — Decision 20
```

`CONTROL` is authoritative for all three. The UI, a console command and any system read and write it through `target_control(id)` / `set_target_control(id, ctl)` / `set_target_level(id, lvl)`, the last of which is a **CAS** so it preserves the sibling bit-fields. There is no ECS mirror, no `dirty`, and no sync system. Change detection is not needed because there is nothing to reconcile.

**Why one packed byte and not three arrays** *(X1)*. The three runtime knobs the game-facing audience needs are delivered **in the register the gate already loaded**: one `Relaxed` byte load, one `and`, one `cmp`. A parallel `SAMPLE_SHIFT` array would cost a second load and a second cache line on the *enabled* path. `.bss`-zero still means level `Off`, shift 0, sync off — the "unbooted is free and correct" property is untouched. This is gated, not assumed: `log_disabled_runtime` must stay **NOT RESOLVED** against the v2 single-level shape in the same sitting (G10d), and if it resolves, the packing is reverted rather than the target being raised.

### Decision 15: Target IDs are compile-time-unique, VALID BY CONSTRUCTION, and cut into three bands *(fixes M15, F15; extended by X2)*
v1 hand-assigned `id = $id:literal` per target with a boot collision check that could only fire if *both* colliders registered — and nothing forced registration, so an unregistered target still gated against `CONTROL[ID]` and never tripped.

**Band map** *(re-cut by X2)*:

| Band | IDs | Uniqueness proof |
|---|---|---|
| Engine | 0..=95 | one `targets! { … }` table; `const _: () = assert!(strictly_increasing)` ⇒ **collisions do not compile** |
| Downstream source | 96..=223 | `define_target!` + boot check `boyko-E0104` naming both colliders |
| **Dynamic** | 224..=255 | minted at runtime from a name (Decision 18); the mint is the uniqueness proof |

Registry check 7 asserts every `LogTarget` impl in the workspace resolves to a table row or a `define_target!` expansion. The honour system is confined to code we do not own, and that is stated.

**`TargetId` is valid by construction; the `pub` field is removed** *(fixes F15)*. v2 declared `pub struct TargetId(pub u16)` against `MAX_TARGETS = 256` with the bound checked only under `debug_assert!`, and made `target_level`/`set_target_level` public. That is either a panic in a hot-path indexing operation or `get_unchecked` **UB reachable from safe public API in release**, and v2 stated neither. v3:

```rust
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)]
pub struct TargetId(u16);        // PRIVATE field — the invariant is `.0 < MAX_TARGETS`
```

The only constructors are (a) `targets!`, (b) `define_target!` — both `const` and both carrying `const _: () = assert!(id < MAX_TARGETS)` — and (c) `register_dynamic_target`, which returns ids from the dynamic band only. There is therefore **no representable out-of-range `TargetId`**, and `CONTROL.get_unchecked(id.0 as usize)` carries a SAFETY comment naming that closed constructor set as the invariant.

**`TargetId::INVALID` is deleted; absence is `Option<TargetId>`.** A public in-band sentinel that indexes an array is the same hazard in a nicer coat. `register_dynamic_target` returns `Option<TargetId>`; a game that has not yet registered stores `None` and cannot call `dyn_info!` at all. This removes v3-draft's `UNREGISTERED_DROPPED` counter, which existed only to count an unreachable state — a counter for an impossible event is a gate that cannot fire.

`boyko_utils::TypeIntern` is **not** usable here: `ID` must be a `const` for gate (a) to fold, and `boyko_utils` depends on `boyko_log`, not the reverse. Recorded so the next reader does not re-derive it.

---

## Key decisions — the scope extension (games as a first-class audience)

### Decision 16: What "as much data as possible" can and cannot mean here
The ask is real, and one common answer to it is wrong for this engine: **enlarging the ring does not raise the capture rate.** The ring's job is to absorb burstiness between a producer offering up to 66 M records·s⁻¹ (15 ns/record) and a consumer formatting at ~500 K·s⁻¹. Enlarging it moves the loss point later; it does not move the *ceiling*, which is `core::fmt` on the sink thread. Four mechanisms actually move the ceiling or make the loss honest, and each is a separate decision below:

1. **Do not format** — `BinarySink` writes `{site_id, tsc_delta, len, flags, payload}` and defers formatting to an offline decoder (Decision 22). This is the only change that moves the throughput ceiling, and it ships **with a revert clause** (G12c): if it does not measure ≥ 5× the text sink in the same sitting, L13b is reverted rather than justified.
2. **Emit less, on purpose, and say so** — per-target sampling (Decision 20), whose census status is `UNPROVEN(sampled)` so a sampled count can never be read as a total.
3. **Keep the loss count honest at session scale** — saturating counters, power-of-two `EveryN`, cursor-wrap correctness (Decision 21). A session is hours; every `u32` in the design was audited against that.
4. **Do not discard the beginning of the capture silently** — rotation reports what it deleted (`W0112`, E21), and `Rotation::NONE` stays the engine default so a bench cannot lose its own start.

**What this plan will not do:** promise a lossless capture. It promises that loss is *counted, attributed to a target, and rendered as a status a reader cannot mistake for a total* — and that promise is gated by G11 and P2 at session scale, not by a 300-frame argument.

### Decision 17: Per-target statistics are the game's read surface — and the census is where a vacuous gate goes to die
`TARGET_STATS: [TargetStatCell; MAX_TARGETS]` (16 KiB `.bss`, one 64 B cell per target, written by the consumer role, readable by anyone) carries `delivered` / `dropped` / `sampled_out` / `sync_routed` as `u64`. `LogCensus` (a `Resource`, `VmColumn`-backed) is its ECS-visible snapshot, refreshed once per drain.

The census status vocabulary is the plan's answer to "a diagnostic whose normal is indistinguishable from its broken":

| Status | Meaning |
|---|---|
| `MEASURED` | records were delivered; the counts are totals |
| `UNPROVEN` | zero records — **never** `clean` |
| `UNPROVEN(lossy)` | `dropped > 0`; the counts are lower bounds |
| `UNPROVEN(sampled)` | the target's shift is non-zero; `delivered` is `1/2^k` of the truth |
| `UNPROVEN(unsunk)` | **no `Active` sink's filter accepts this target** — a game enabled a category, saw nothing, and would have concluded "clean". `boyko-W0111` fires once |
| `dropped=SATURATED(>=4294967295)` | the per-lane `u32` hit its ceiling; not a number a reader may compare |

`LogCensus.lossy` is the single bit a UI must read before rendering any count as a total, and `G15` gates that the bit exists and flips.

### Decision 18: Dynamic targets — 32 slots, interned by name, and the cost of losing gate (a) is stated
A game or mod names a category from data (`"mod:acme_weapons"`, a script namespace, a save-file field). `register_dynamic_target(name, initial) -> Option<TargetId>` is **cold, setup-time and idempotent by name**: it hashes into `DYN_NAMES`, an open-addressed, insert-only, fixed-capacity table of 32 cache-line slots in `.bss`. Not a map: no rehash, no growth, no allocation, and **the emission path never touches it** — emission carries the `TargetId`, and the name is resolved by the sink.

Emission uses `dyn_info!(id, …)` / `dyn_warn!(id, code, …)`, which have **two** gates instead of three: `T::STATIC_CEILING` does not exist for a target that is not a type. The cost is real and is not smoothed over: a dynamic site cannot be compiled out per-target, only by `GLOBAL_CEILING`. The bench `log_dyn_disabled` bounds it at ≤ 4 ns, and **G8d turns the comparison into a claim that can be withdrawn**: if `log_dyn_disabled − log_disabled_runtime` does **not** resolve above the sitting's floor, then the per-target `const` ceiling's benefit is unproven on this box and Decision 2's claim about gate (a) is **struck from this document** rather than restated.

**Why 32 and not "unbounded"** — see open question 8. Every slot comes out of the 256-target space that `CONTROL`, the sink filters (`[u64; 4]`) and `TARGET_STATS` are all sized by; past 256 those three arrays become two-level structures. 32 data-defined categories is a lot; needing more is a signal that the taxonomy belongs in source.

### Decision 19: Downstream code tables — the same macro, a different prefix, and a lazily-minted dense index
`codes!` is exported with a `prefix` parameter. A game invokes it once (`prefix = "acme"`, `doc_root = "docs/diag"`), gets its own `pub const` per code and its own `DiagInfo` table, and invokes `codes_tidy!(root = …, prefix = …)` to generate **the same eight checks over its own corpus** — because the engine's checks prove nothing about a game's registry, and that sentence is in G9's assertion message, not only in this document.

The `RATE` index must stay dense. Engine codes carry a compile-time `CodeIdx::Static(u16)`; downstream codes carry `CodeIdx::Dynamic(&'static AtomicU16)`, minted on first use with the reserve-then-publish protocol (`CAS UNASSIGNED→RESERVED`, `fetch_add` on `CODE_OCCUPANCY`, `store(Release)`), so 16 threads racing on one code produce exactly one index and leak none (G9). Cost on the downstream `Warn`/`Error` path only: one extra `Relaxed` load and one predicted-not-taken branch (~1 ns, measured by `downstream_code_warn` against the engine-code `warn!` in the same sitting). `CODE_OCCUPANCY` past 90 % emits `boyko-W0114`.

**Decision 7 is NOT relaxed for games**: `Warn`/`Error` still MUST carry a code, and a code is still a promise of a documented page. Data-defined *codes* are refused (§Refused) precisely because a data-defined code cannot have one.

### Decision 19b: `LogPod` — game types as arguments, without weakening Decision 13
```rust
pub unsafe trait LogPod: Copy + Send + Sync + 'static {
    const POD_LEN: usize;                                   // == size_of::<Self>()
    fn fmt_pod(bytes: &[u8], f: &mut LogFormatter);         // runs on the SINK
}
```
Blanket-bridged into `LogValue`, so the sealing of `LogValue` is preserved. **The encode half is ours** — a `copy_nonoverlapping` of `POD_LEN` bytes — and the user's `fmt_pod` runs on the **sink thread, from the staging arena, in the same position as `site.decode`**. Decision 13's structural property therefore holds unchanged: no user code runs between lane acquisition and the `Release` store. This is asserted, not argued: test 24 uses a `LogPod` whose `fmt_pod` sets a TLS flag and requires the flag to be **unset** at the `Release` store and set only during drain.

`#[derive(LogPod)]` in `boyko_macros` requires `#[repr(C)]` and all-`LogValue` fields (no new dependency edge: `boyko_macros` is a proc-macro crate with no `boyko_ecs` dependency). A hand-written `unsafe impl` is allowed and carries the usual SAFETY burden; G9b catches a lying `POD_LEN` under Miri but **cannot make an arbitrary hand impl safe**, and says so.

The `*_kv!` macros (`info_kv!(Combat, "hit", dmg = d, target = t)`) put field **names** in the `&'static LogSite`, which is cold and never touched on the emission path — so structured output costs the same as positional output on every hot path.

### Decision 20: Sampling and sync-routing — two bits of `CONTROL`, both default-off, both gated with a revert clause
**Sampling.** `k = (ctl >> 3) & 0x0F`; when `k != 0`, deliver 1 record in `2^k`. The counter is `SAMPLE_CTR[lane][target]`, a `u16` **written only by the lane's owner** (the row index *is* the lane index), with plain `Relaxed` load/store and **never an RMW** — so it inherits the `Lane` SAFETY block's single-writer clause verbatim and costs no lock prefix. Seeded at claim time with `(lane * 0x9E37)` so two lanes do not phase-lock.

**What sampling cannot claim**: that the capture is *representative*. `1/2^k` is **strided, not random**; a periodic emitter aliased to `2^k` yields a systematically biased capture. The census prints `sampling=1/N (strided, not random)`, `boyko-W0113` fires once per sampled target, and E23 states the residual. A footnote nobody reads is not a control; a line in the log is.

**Sync routing.** Bit 7 routes a target's records to the synchronous channel: format on the caller, `write_oracle_line`, count `sync_routed`. ~200+ ns and it serialises the frame — that is the *point*: it is the per-target opt-in for "this must be on disk before the next instruction", which is the only partial answer to a hard crash (E22).

Both branches are predicted-not-taken in every default configuration. **G10d decides whether sampling ships default-on**: `log_enabled_0args` must be NOT RESOLVED against the pre-L12 baseline; if it resolves, `log-sampling` becomes a default-off feature and the ≤ 15 ns row is annotated with the measured cost. The gate decides the rung's disposition; this document does not pre-decide it.

### Decision 21: The session-scale integer audit
A 300-frame bench cannot distinguish a correct counter from one that wraps in 65 seconds. Every integer was audited against an hours-long session:

| Quantity | Width | Behaviour at the limit | Where |
|---|---|---|---|
| `Lane::write` / `read` | `u32` byte cursors | **Wraps, correctly.** Every comparison is `wrapping_sub`, every index is `& MASK`, and `w − r ≤ LANE_BYTES ≪ 2³¹`, so the unsigned difference is unambiguous across a wrap. Wrap arrives in ~2.4 h at 500 KB·s⁻¹·lane | E17, test 19 |
| `dropped`, `dropped_bytes` | `u32` | **Saturate**, census prints `SATURATED` | E18, G11 |
| `RateSlot::count` | `u32` | Wraps; harmless **only because `EveryN(n)` is now power-of-two** (X3) | Decision 8 |
| `LogStats.*`, `TargetStat.*` | `u64` | ~584 years at 1 G·s⁻¹ | — |
| `LogRing::head`, `arena_cursor`, `seq` | `u32` / `u64` | Wrap-correct; `seq` is the reader's cursor and a gap is reported as `skipped` | test 20 |
| `tsc` | `u64` | ~195 years | E8 |

### Decision 22: `BinarySink` — the only mechanism that raises the ceiling, shipped with a revert clause
The sink writes `{site_id, tsc_delta, len, flags, payload}` with **no formatting**; `site_id` comes from `SITE_DICT`, a sink-thread-only open-addressed `*const LogSite -> u32` table (64 KiB `.bss`), with a dictionary record emitted on a `#[cold]` miss. `logdec` (a small bin) replays the dictionary and formats offline. Format and `schema_version` are pinned in `docs/LOG-BINARY-FORMAT.md`; the decoder **refuses** a mismatch rather than best-efforting it. Every rotated file re-emits the anchor and dictionary so it decodes standalone.

**Revert clause (G12c)**: the entire justification is throughput. If `sink_sustained_rate_binary` does not measure ≥ 5× `sink_sustained_rate` in the same sitting, **L13b is reverted**. A format whose only reason to exist is speed must show the speed.

### Decision 23: Runtime control with no restart, no lock, and no I/O on the caller's thread
- **Levels / sampling / sync**: a `CAS` on one `CONTROL` byte from any thread. `CONTROL_EPOCH` is a `Release` counter a UI polls to know it must repaint — an `O(1)` substitute for the change detection Principle 0's refused ECS route would have given (see §Refused).
- **Sink state / filter / floor**: plain byte stores into `SinkSlot` from any thread. A sink acts on the filter it read at the top of its **current** drain, so a change lands within one drain — a stated property, pinned by G13, not hidden.
- **Sink lifecycle (open / close / retarget)**: goes through `SINK_REQ`, a 16-entry `.bss` ring written under `OUT_LOCK`, consumed by the sink thread. **No `open`, no allocation and no syscall ever runs on the requesting thread** — G13b proves it with the per-thread counting allocator. A full queue is `boyko-E0107`, never a silent drop. A channel was rejected: it is an allocation and usually a `Mutex`.
- **`apply_control_spec("net=debug/6!, ecs=off")`** parses a console/env/file spec, applies it with one `CONTROL_EPOCH` bump, leaves unnamed targets **bit-identical**, and rejects an unknown name with a coded error rather than ignoring it (test 30).

**Capability vs state, as the project rule requires**: a category *exists* because a `LogTarget` (or a dynamic registration) exists — structural. It is *on or off* by a bit in `CONTROL` — state. The rule's substance is honoured at the layer that can afford it; §Refused records why `CONTROL` is not an ECS column and what that costs.

### Decision 24: The crash drain takes the consumer role by CAS, or does nothing
The panic hook (chained ahead of the existing hook) writes the panic message synchronously, then `flush()`. If `flush()` cannot succeed, it attempts `SINK_STATE.compare_exchange(from, CrashDraining)` for `from ∈ {Exited, NotBooted, Manual}` — **the three states in which no sink thread can be inside a drain**. Only on success does it run the drain itself, into the `CrashSink` (a file **opened at boot**, because opening a file inside a panic hook is its own failure mode), and emit `boyko-E0109`. If `SINK_STATE` is `Running` or `Exiting`, it **returns without draining**: displacing a live sink would put two consumers on one lane, which the `Lane` SAFETY block forbids. Termination is a bounded loop over three constants; no wait is added.

**What it cannot do**: survive `abort()`, `SIGSEGV`, or a guard-page stack overflow — the hook does not run. Stated in E22 and in G14's "cannot claim" column, with the partial mitigations named (the per-target sync bit, `flush_interval_ms`, and a crash file that at least exists and carries the session header).

### Decision 25: Four profiles, chosen by the host, printed in every header
| Profile | `GLOBAL_CEILING` | Lanes | Sinks | Sampling | Intended for |
|---|---|---|---|---|---|
| `dev` | `Trace` | 128 × 16 KiB | console + file, `Rotation::NONE` | off | engine work, benches, goldens |
| `shipping` | `Info` | 32 × 16 KiB | binary + crash, rotation on | opt-in | a released title |
| `shipping-min` | `Warn` | 32 × 16 KiB | crash only, `SinkMode::Manual` | opt-in | a title that wants no resident sink thread |
| `off` | `Off` | 0 | none | — | G2's leg; a true off switch |

The profile name and the 128-bit `SessionId` appear in every sink header, so an uploaded log identifies what was compiled in. **G16** is a two-sided symbol gate: no `emit_impl` monomorphisation reachable from a `debug!`/`trace!` fixture may appear in the `shipping` binary, and it **must** appear in `dev`. Its fixture includes a `dyn_debug!` site, because a dynamic site has no gate (a) and `GLOBAL_CEILING` is the only thing that deletes it.

### Decision 26: In-frame consumption — the game reads its own diagnostics, one drain behind
`LogRing::since(cursor, &RingFilter) -> LogRingIter` returns records delivered since a monotone `seq`, oldest first, with `LogRingIter::skipped` reporting how many the ring wrapped past — **a console cannot silently miss lines**. The ring is fed by `log_drain_system` in `Last`, from the sink's handoff, never from the emission path: **G15b reds if a record is visible before the drain that consumed it**, which is also what keeps the hot path from touching ECS storage.

The stated bound is "sink park interval + one frame" (≤ 2 frames in practice), and G15 cannot claim tighter. A per-frame `EPOCH` record lets a reader attribute every record to exactly one frame; a record emitted *during* the drain is attributed to the next frame, and test 29 asserts that rather than assuming it.

---

## Audience conflicts — named, decided, and costed for the losing side

Five places where the engine's needs and a game's needs genuinely pull in opposite directions. Each is decided; each records what the losing side gives up.

| # | Conflict | Decision | **Cost to the losing side** |
|---|---|---|---|
| **C1** | **Compile-time categories** (three gates, one const-folded per target, zero cost when off) vs **data-defined categories** (a mod's name is not a Rust type) | **Both, in separate bands.** Static targets keep all three gates; dynamic targets (224..=255) have two | A dynamic site **cannot be compiled out per-target** — only `GLOBAL_CEILING` deletes it. Bounded at ≤ 4 ns disabled, and **G8d can strike the claim that gate (a) matters at all** if the difference does not resolve on this box |
| **C2** | **"As much data as possible"** vs **a fixed-capacity ring that drops** | The ring stays fixed; the answer to volume is **not formatting** (`BinarySink`) plus **honest loss accounting at session scale** | A game cannot have a lossless capture from this crate. It gets: a counted, per-target, saturation-aware loss report and a `lossy` bit; and `sink_sustained_rate`'s knee measured, not asserted. Enlarging the ring is explicitly refused (§Refused) because it moves the loss point without moving the ceiling |
| **C3** | **Runtime toggling from a console / dev menu / remote** vs **no lock and one authority on the hot path** | One **packed `CONTROL` byte**, CAS-written from any thread, read `Relaxed` by the gate. Sink lifecycle goes through `SINK_REQ`; no I/O on the caller | No `Query`, no change detection, no `EnableTag` over `CONTROL` — it is not an ECS column, and cannot be (Principle 0 exception, Invariant 7). Mitigation is `CONTROL_EPOCH`, a poll not a subscription. Filter changes land within one drain, not instantly (G13's stated limit) |
| **C4** | **What ships in a released title** (small, quiet, crash-durable) vs **what the engine needs** (loud, complete, byte-frozen) | **Four profiles** (Decision 25), selected by the host, printed in every header | `shipping` compiles out `debug!`/`trace!` entirely — a support ticket cannot ask a player to "turn on trace" without a different build. `shipping-min` has **no resident sink thread** and therefore no in-frame HUD |
| **C5** | **Gameplay reading its own diagnostics in-frame** vs **the drain staying off the frame thread** | `LogRing` is fed by `log_drain_system` in `Last` from the sink's handoff; the reader is a cursor + filter | The reader is **one drain + one frame behind**, and gameplay **may not branch on log counters** (§Refused: they are lower bounds under drop, schedule-dependent, and break replay). Display and telemetry only |

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
    pub code_idx: CodeIdx,      // 4  DENSE registry index (M12); Static or Dynamic (D19)
    pub line:     u32,          // 4
    pub file:     &'static str, // 16
    pub fmt:      &'static str, // 16 the format literal, printed by the sink
    /// Field names for the `*_kv!` forms; empty for the positional forms.
    /// `&'static`, cold, NEVER touched on the emission path — which is why
    /// structured output costs the same as positional output (D19b).
    pub fields:   &'static [&'static str],
    pub prefix:   &'static str, // "boyko" for the engine; a game declares its own (D19)
    /// Monomorphised per ARGUMENT-TUPLE type; identical tuples share one
    /// instantiation. Cold: called on the sink thread only.
    pub decode:   unsafe fn(*const u8, usize, &mut LogFormatter),
}
// The per-SITE `Once` latch is NOT a field here: `LogSite` is immutable
// `&'static` data. The macro expands a sibling `static FIRED: AtomicBool`
// beside each `Once` site — per-site, private line, 1 byte (Decision 8, F11).

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
    /// Absolute WRAPPING byte counter; `off = write & MASK`. `Release`-stored;
    /// this store is the happens-before edge that publishes the payload.
    /// Wrap at 2^32 bytes (~2.4 h at 500 KB/s) is CORRECT, not merely unlikely:
    /// every comparison is an unsigned wrapping difference and every index is
    /// masked (Decision 21, E17, test 19).
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
    /// SATURATING at u32::MAX — a counter that wraps is a counter that lies,
    /// and the only state in which it can grow unbounded is "no drain ever"
    /// (Decision 5, Decision 21, G11).
    dropped:       AtomicU32,  // 4  Relaxed; cleared with fetch_sub(observed)
    dropped_bytes: AtomicU32,  // 4  saturating, same argument
    /// Deliberately NOT emitted (Decision 20). NOT a loss — counted separately
    /// so that conflating the two cannot make either number a liar. The
    /// property test `emitted == drained + dropped + sampled_out` depends on
    /// the separation being exact.
    sampled_out:   AtomicU32,  // 4
    /// FREE / RETIRING / owner token. `load`-then-CAS on claim (M10).
    owner:         AtomicU32,  // 4
    _pad2:         [u8; 48],

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
//   1c. `LogPod` does not weaken 1b: the ENCODE half is our blanket
//      `copy_nonoverlapping` of POD bytes, and the user-supplied `fmt_pod`
//      runs on the SINK thread from the staging arena, in the same position as
//      `site.decode` (Decision 19b, asserted by test 24).
//   1d. `SAMPLE_CTR[lane]` is written only by the lane's owner (the ROW INDEX
//      IS THE LANE INDEX), with Relaxed load/store and never an RMW, so it
//      inherits clause 1 verbatim (Decision 20).
//   1e. `read_cached` is a `Cell<u32>` READ AND WRITTEN ONLY BY THE PRODUCER
//      that owns the lane — it is the reason `Lane` is not `Sync` by
//      derivation, and clause 1's exclusive-write argument covers it exactly:
//      a second producer thread cannot exist, and the consumer never names it.
//      Its value is a STALE lower bound on `read`; staleness is safe because
//      it can only make the producer refuse space it actually had, never grant
//      space it did not (Decision 5's induction) (F23).
//   1f. `write_cached` is the mirror clause: `Cell<u32>`, read and written
//      only by the thread currently holding the CONSUMER role (clause 2), and
//      a stale value can only make the consumer drain less than was available,
//      never read past `write` (F23).
//   2. READ side: exactly one thread reads `buf` and writes `read`. Normally
//      the sink; during a crash drain the panicking thread, and ONLY after a
//      CAS of SINK_STATE out of {Exited, NotBooted, Manual} — the three states
//      in which no sink thread can be inside a drain (Decision 24).
//      `SinkMode::Manual` documents `drain()` as single-caller and asserts it.
//   3. Payload visibility: bytes written before `write.store(_, Release)` are
//      visible to a thread observing that value via `Acquire`. The consumer
//      never reads past its observed `w`, AND never advances `read` over bytes
//      it has not yet copied out (Algorithms C).
//   4. Retire: the TLS guard's `Drop` runs on the producer thread after its
//      last write; the consumer stores FREE only after observing
//      `RETIRING && read == write`, so no producer write can follow a reclaim.
unsafe impl Sync for Lane {}

pub(crate) const MAX_LANES:  usize = 128;        // option_env!-overridable (32 in `shipping`)
pub(crate) const LANE_BYTES: usize = 16 * 1024;  // power of two: MASK arithmetic
const MASK:          u32   = (LANE_BYTES - 1) as u32;
/// Usable span. ONE slot reserved so `used == CAPACITY` cannot be confused with
/// `used == 0` without a third variable — and, critically, so that
/// `avail = CAPACITY - used` in Algorithms A6 cannot underflow (F6).
const CAPACITY:      u32   = (LANE_BYTES - 1) as u32;
const ERROR_RESERVE: u32   = (LANE_BYTES / 8) as u32;   // 2 KiB, Error-only tail
const _: () = assert!(LANE_BYTES.is_power_of_two());
const _: () = assert!(ERROR_RESERVE < CAPACITY);        // else no non-Error record ever fits

pub const LANE_ARRAY_LEN: usize = if (GLOBAL_CEILING as u8) == 0 { 0 } else { MAX_LANES };
static LANES: [Lane; LANE_ARRAY_LEN] = [Lane::NEW; LANE_ARRAY_LEN];

// ────────────────────────── boyko_log/src/target.rs ───────────────────────────

/// PRIVATE field: the invariant `.0 < MAX_TARGETS` is upheld by a CLOSED set of
/// three constructors (`targets!`, `define_target!`, `register_dynamic_target`),
/// which is what makes the hot-path `get_unchecked` sound. v2's `pub u16` made
/// out-of-range values constructible from safe code (F15). There is no
/// `INVALID` sentinel — absence is `Option<TargetId>`.
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)] pub struct TargetId(u16);

pub trait LogTarget: 'static {
    const NAME: &'static str;
    const ID: TargetId;           // const: required for gate folding
    const STATIC_CEILING: Level;  // const: Unreal's compile-time ceiling
}
pub const MAX_TARGETS:    usize = 256;
pub const DYN_BAND_START: usize = 224;                          // Decision 15's re-cut band
pub const DYN_BAND_LEN:   usize = MAX_TARGETS - DYN_BAND_START; // 32

/// ONE packed byte per target: level | sample shift | sync route (Decision 14).
/// One `Relaxed` byte load + `and` + `cmp` per enabled-check. 256 B; one line.
/// `.bss`-zero == level `Off`, shift 0, sync off == disabled until boot arms it.
static CONTROL: [AtomicU8; MAX_TARGETS] = [const { AtomicU8::new(0) }; MAX_TARGETS];
/// Monotone; `Release`-added on every control change. A UI POLLS this to know
/// it must repaint — the O(1) stand-in for the change detection the refused ECS
/// route would have given (§Refused, Decision 23).
static CONTROL_EPOCH: AtomicU32 = AtomicU32::new(0);

/// ONE pointer publishes name+len together — v1's `AtomicPtr<u8>` lost the
/// length of a `&'static str` (N28). Read by the sink and by `targets()` only.
pub struct TargetInfo { pub name: &'static str }
static TARGETS: [AtomicPtr<TargetInfo>; MAX_TARGETS] = ...;

/// Dynamic-target name interning. Open-addressed, insert-only, fixed capacity,
/// one cache line per slot. NOT a map: no rehash, no growth, no allocation, and
/// the emission path never touches it (Decision 18). 2 KiB .bss.
#[repr(C, align(64))]
struct DynSlot { hash: AtomicU64, len: AtomicU8, bytes: UnsafeCell<[u8; 47]> }
static DYN_NAMES: [DynSlot; DYN_BAND_LEN];
// SAFETY: `bytes`/`len` are written before `hash.store(h, Release)`; a reader
// that observes a non-zero hash via Acquire observes the completed name. A
// slot's hash transitions 0 -> h exactly once, by CAS. No slot is ever reused,
// so a published name is immutable for the process lifetime.

/// Sink-written, anyone-readable. NOT a mirror of anything: it is the only
/// place delivered-per-target counts exist (Decision 17). 16 KiB .bss.
#[repr(C, align(64))]
struct TargetStatCell { delivered: AtomicU64, dropped: AtomicU64,
                        sampled_out: AtomicU64, sync_routed: AtomicU64,
                        _pad: [u8; 32] }
static TARGET_STATS: [TargetStatCell; MAX_TARGETS];

/// One u16 row per LANE, one column per target. Written ONLY by the lane's
/// owner with plain Relaxed load/store, never an RMW (Decision 20, SAFETY 1d).
static SAMPLE_CTR: [[Cell<u16>; MAX_TARGETS]; MAX_LANES];   // 64 KiB .bss

// ────────────────────────── boyko_log/src/codes.rs ────────────────────────────

#[repr(C)] #[derive(Clone, Copy)] pub struct WarnCode  { num: u16, idx: CodeIdx }
#[repr(C)] #[derive(Clone, Copy)] pub struct ErrorCode { num: u16, idx: CodeIdx }
#[repr(C)] #[derive(Clone, Copy)] pub struct PanicCode { num: u16, idx: CodeIdx }
// Distinct newtypes ⇒ `warn!(T, codes::E2101, ..)` does not compile.

/// Engine codes carry a compile-time dense index; downstream codes carry a
/// pointer to a lazily-minted cell (Decision 19). Cost: one extra Relaxed load
/// and one predicted-not-taken branch, on the DOWNSTREAM Warn/Error path only.
#[derive(Clone, Copy)]
pub enum CodeIdx { Static(u16), Dynamic(&'static AtomicU16) }

#[repr(u8)] pub enum RatePolicy { Every, Once, OnceCounted, EveryN(u16), MinIntervalMs(u16) }
// `codes!` emits `const _: () = assert!(n.is_power_of_two())` for EveryN, so
// `count & (n-1)` is exact across a u32 wrap (Decision 8, Decision 21).
// `Once`/`OnceCounted` do NOT use `RATE` at all — the latch is per SITE (F11).

/// Registry ROW STATUS — the mechanism that lets L2 commit alone on a
/// grandfathered corpus (Decision 6, F20). `Pending` rows must have ZERO
/// emitters (check 3b); `Live` rows must have at least one (check 3).
#[derive(Clone, Copy, PartialEq, Eq)] pub enum CodeStatus { Live, Pending }

pub struct DiagInfo {
    pub number: u16, pub class: u8,
    pub prefix: &'static str,    // "boyko" for the engine; games declare their own
    pub summary: &'static str,   // one line, embedded, printable from a message
    pub rate: RatePolicy,
    pub status: CodeStatus,
    pub doc: &'static str,       // "docs/diagnostics/W1501.md" — check 2's target
}
static DIAGNOSTICS: [DiagInfo; N];       // dense, sorted; index == code_idx
const MAX_CODES: usize = 512;
static CODE_OCCUPANCY: AtomicU16;        // downstream minting; W0114 at 90 %

/// 64 B — one code per cache line. v1's 16 B slot false-shared four unrelated
/// subsystems' codes on one line (Decision 8).
#[repr(C, align(64))]
struct RateSlot { fired: AtomicBool, count: AtomicU32, last_tsc: AtomicU64, suppressed: AtomicU32, _pad: [u8; 43] }
static RATE: [RateSlot; MAX_CODES];      // 32 KiB .bss

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

// The three consumer-role scratch buffers. ALL THREE ARE `.bss` STATICS, not
// heap — v2 left `STAGE_BYTES`'s backing store unspecified, and "no `Vec`/`Box`
// in any SIGNATURE" is narrower than the claim a reader takes from it (F25).
// They are counted in Decision 3's budget matrix.
static STAGE:     UnsafeCell<[u8; STAGE_BYTES]>;       // 256 KiB — Algorithms C
static SITE_DICT: UnsafeCell<[SiteDictEntry; 4096]>;   // 64 KiB — binary sink only
static SINK_OUT:  UnsafeCell<[u8; 1 << 20]>;           // 1 MiB — binary write buffer
// SAFETY: all three are touched only by the thread currently holding the
// consumer role — the sink thread, or the crash-draining thread after the
// SINK_STATE CAS proved the sink thread cannot be inside a drain (Decision 24).

// ─────────────────── boyko_ecs seam: the ECS-visible surface ──────────────────

/// The durable, displayable log. Backed by the engine's own storage — a
/// `VmReservation`-backed byte column, NOT a `Box<[u8]>` heap side-store, which
/// is the shape Principle 0 was re-stated to forbid even inside a `Resource`
/// (M13). Fixed capacity, reserved at plugin build, never grows.
#[derive(Resource)]
pub struct LogRing {
    lines: VmColumn<LogLine>,  // engine storage
    arena: VmColumn<u8>,       // engine storage
    head: u32, len: u32, arena_cursor: u32,   // wrapping; Decision 21, test 20
    seq:  u64,                 // monotone record sequence — the reader's cursor
}

/// EXACTLY 16 BYTES, `Copy`, and pinned by a const assert *(fixes F7)*.
/// `crates/boyko_ecs/src/ecs/memory/vm_column.rs:144-149` panics in
/// `VmColumn::<T>::new` unless `COMMIT_GRANULE % size_of::<T>() == 0`, and
/// `COMMIT_GRANULE == 64 KiB` (`crates/boyko_ecs/src/ecs/constants.rs:7`).
/// v2's layout was 12 bytes (10 payload + align-4 tail): `65536 % 12 == 4`, so
/// `LogPlugin::build` would have PANICKED at construction and rung L5 could not
/// have landed green. `repr(C, packed)` does not save it either (`65536 % 10
/// == 6`). The fix is a size that divides the granule, not an attribute — and
/// the const assert below turns "someone adds a field" from a plugin-build
/// panic into a compile error. `VmColumn<T>` also requires `T: Copy`, hence the
/// derive; it is `pub(crate)` to `boyko_ecs` (`vm_column.rs:80`), which is fine
/// because `LogRing` lives inside `boyko_ecs` and the field is private.
#[repr(C)] #[derive(Clone, Copy)]
pub struct LogLine {
    start:  u32,   // offset into `arena`
    seq_lo: u32,   // low half of the record's sequence number
    len:    u16,   // bytes of formatted text
    code:   u16,   // 0 when the level carries none
    level:  u8,
    target: u8,    // MAX_TARGETS == 256 fits a u8 exactly
    flags:  u8,    // STR_TRUNCATED | SUPPRESSED_FOLLOWS | SAMPLED_CONTEXT
    _pad:   u8,
}
const _: () = assert!(core::mem::size_of::<LogLine>() == 16);
const _: () = assert!(COMMIT_GRANULE % core::mem::size_of::<LogLine>() == 0,
    "LogLine must divide COMMIT_GRANULE or VmColumn::new panics (F7)");
// `VmColumn<u8>` for the arena is trivially fine: 65536 % 1 == 0.

/// Monotonic counters; zero per-frame allocation. Mirrors the shape of
/// `crates/boyko_app/src/window_info.rs:34`'s `HostFrameStats` — counters only,
/// no timing fields, because that struct HAS no timing fields (checked).
#[derive(Resource, Clone, Copy, Default)]
pub struct LogStats {
    pub emitted: u64, pub dropped: u64, pub dropped_bytes: u64,
    pub suppressed: u64, pub unlaned_dropped: u64, pub sampled_out: u64,
    pub lanes_claimed: u32, pub lanes_retired: u32,
}

/// Per-target counts for an in-game overlay / support HUD / telemetry payload.
/// `lossy` is the one bit a UI must read before showing a count as a total
/// (Decision 17).
#[derive(Resource)]
pub struct LogCensus {
    per_target: VmColumn<TargetStat>,   // MAX_TARGETS rows, engine storage
    pub session: SessionId,             // 128-bit (Decision 25)
    pub lossy: bool,
    pub epoch: u32,                     // CONTROL_EPOCH at the last drain
}
#[repr(C)] #[derive(Clone, Copy)]
pub struct TargetStat { pub delivered: u64, pub dropped: u64,
                        pub sampled_out: u64, pub sync_routed: u64 }
const _: () = assert!(core::mem::size_of::<TargetStat>() == 32);
const _: () = assert!(COMMIT_GRANULE % core::mem::size_of::<TargetStat>() == 0);
// NOTE: there is no `LogFilter`. `CONTROL` is the single owner (Decision 14).
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
/// DOWNSTREAM SOURCE targets: IDs 96..=223, boot-checked (`boyko-E0104`).
#[macro_export] macro_rules! define_target { ($vis:vis $Ty:ident, name=$n:literal, id=$id:literal, ceiling=$c:expr) => {...} }

/// DYNAMIC targets: IDs 224..=255, minted from data/mod/script names.
/// COLD, setup-time, idempotent by name. `None` => band exhausted (`boyko-E0106`).
pub fn register_dynamic_target(name: &str, initial: TargetControl) -> Option<TargetId>;
pub fn find_target(name: &str) -> Option<TargetId>;         // #[cold], linear scan
pub fn targets() -> TargetIter<'static>;                    // #[cold], settings screens

// ── emission: static targets (gates a+b const-folded) ─────────────────────────
#[macro_export] macro_rules! error { ($T:ty, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! warn  { ($T:ty, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! info  { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! debug { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! trace { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }

/// Named-field form; names live in the cold `LogSite`, so this costs the same
/// as the positional form on every hot path (Decision 19b).
/// `info_kv!(Combat, "hit", dmg = d, target = t)`
#[macro_export] macro_rules! info_kv  { ... }   // + debug_kv! trace_kv! warn_kv! error_kv!

// ── emission: dynamic targets (gate (a) unavailable — Decision 18) ────────────
#[macro_export] macro_rules! dyn_info  { ($id:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! dyn_warn  { ($id:expr, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
// + dyn_debug! dyn_trace! dyn_error!

/// Renders a `Display` into a stack buffer owned by the ARGUMENT EXPRESSION.
/// Expansion form pinned in Decision 13 — the naive block form does not
/// borrow-check (F22).
#[macro_export] macro_rules! dsp { ($e:expr) => {...}; ($e:expr, $n:literal) => {...} }

/// SYNCHRONOUS, ordered, never dropped, never rate-limited, no code, stdout,
/// under OUT_LOCK. Machine-parsed measurement lines ONLY (Decision 9).
#[macro_export] macro_rules! report { ($fmt:literal $(, $a:expr)*) => {...} }

/// SYNCHRONOUS, ordered, never dropped, stderr, under the SAME OUT_LOCK, with
/// the BOUNDED acquire of Decision 9c. Callers: the lane-exhaustion fallback
/// for Warn/Error (Decision 5), sync-routed targets (Decision 20), the panic
/// hook and flush's timeout path. NOT the Vulkan messenger — that site is not
/// touched at all (Decision 9b). Not for ordinary diagnostics.
pub fn write_oracle_line(prefix: &str, body: &[u8]);

// ── values ────────────────────────────────────────────────────────────────────
pub trait LogValue: private::Sealed {
    const MAX_ENCODED_LEN: usize;
    fn encoded_len(&self) -> usize;
    unsafe fn encode(&self, dst: *mut u8) -> usize;
}
// impls: {i,u}{8,16,32,64,128}, f32, f64, bool, char, &'static str, &str.

/// Game-extensible POD values. Blanket-bridged into `LogValue`, so sealing is
/// preserved and Decision 13's structural property is untouched: the encode
/// half is ours, `fmt_pod` runs on the sink (Decision 19b, test 24).
pub unsafe trait LogPod: Copy + Send + Sync + 'static {
    const POD_LEN: usize;
    fn fmt_pod(bytes: &[u8], f: &mut LogFormatter);
}
// boyko_macros: #[derive(LogPod)] — requires #[repr(C)] and all-LogValue fields.

// ── codes ─────────────────────────────────────────────────────────────────────
/// Engine registry (single invocation, `prefix = "boyko"`), and the SAME macro
/// exported for downstream tables (Decision 19).
#[macro_export] macro_rules! codes { (prefix = $p:literal, doc_root = $d:literal, $($row:tt)*) => {...} }
/// Generates the EIGHT registry checks over a caller-supplied root+prefix.
/// The engine's own checks prove nothing about a downstream crate.
#[macro_export] macro_rules! codes_tidy { (root = $r:literal, prefix = $p:literal) => {...} }

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
    pub sink_mode: SinkMode,            // Thread (default) | Manual
    pub console:   Option<ConsoleSink>, // stderr; stream/colour/level floor
    pub file:      Option<FileSink>,    // path, rotation
    pub binary:    Option<BinarySink>,  // path, rotation, flush_interval_ms
    pub crash:     Option<CrashSink>,   // path; OPENED AT BOOT (Decision 24)
    pub callback:  Option<CallbackSink>,// extern "C" fn(&FormattedRecord, *mut ()) + ctx
    pub ecs_ring:  bool,
    pub census:    CensusPolicy,        // OnFlush (dev) | OnShutdown | Interval(secs)
    pub control_source: ControlSource,  // None | Env | File(&'static str)
    pub default_controls: [TargetControl; MAX_TARGETS],
}
pub struct Rotation { pub max_bytes: u64, pub keep: u8 }   // Rotation::NONE = v2 behaviour
pub enum LogBootError { AlreadyBooted, TargetIdCollision { id: u16, a: &'static str, b: &'static str }, SinkOpen(std::io::Error) }
pub enum FlushResult { Flushed, NoConsumer, TimedOut }

pub fn boot(cfg: LogConfig) -> Result<(), LogBootError>;  // no handle (Decision 12)
pub fn shutdown();                                        // idempotent
pub fn flush() -> FlushResult;                            // never waits on a dead consumer
pub fn drain();                                           // SinkMode::Manual only
pub fn session_id() -> SessionId;
pub fn explain(code: u16) -> Option<&'static DiagInfo>;
pub fn census() -> CensusIter<'static>;                   // MEASURED / UNPROVEN per target
pub fn name_current_thread(name: &'static str);           // cosmetic, cold, once

// ── ECS seam (boyko_ecs) ──────────────────────────────────────────────────────
pub struct LogPlugin { pub config: LogConfig }
impl Plugin for LogPlugin { fn build(&self, app: &mut App); }
// inserts LogRing / LogStats / LogCensus; adds `log_drain_system` to `Last`
// (ECS ring feed + TARGET_STATS snapshot + one EPOCH record per frame — the
// sink thread owns the byte sinks). Registers `shutdown` on teardown.

impl LogRing {
    /// Records delivered since `cursor`, oldest first. `cursor` is a monotone
    /// sequence number; a gap means the ring wrapped and `LogRingIter::skipped`
    /// says by how much — a console cannot silently miss lines (Decision 26).
    pub fn since(&self, cursor: u64, filter: &RingFilter) -> LogRingIter<'_>;
    pub fn cursor(&self) -> u64;
}
pub struct RingFilter { pub targets: [u64; 4], pub min_level: Level }
```

No `Vec`, `Box<dyn>`, `HashMap` or internal type appears in any signature. The callback seam is an `extern "C" fn` + ctx, so it crosses a dylib boundary with no vtable and no allocation.

---

## Algorithms for critical paths

### A. `emit` — the producer hot path

```
1. GATE (inlined into the caller)
   a. T::STATIC_CEILING       >= LVL           — const, folded  [ABSENT for dyn_* — D18]
   b. $crate::GLOBAL_CEILING  >= LVL           — const, folded
   c. ctl = CONTROL[T::ID].load(Relaxed);  (ctl & 0x07) >= LVL  — 1 B L1 load + and + cmp
   Fail ⇒ nothing. Arguments NEVER evaluated (&& short-circuit).
   [Arguments, incl. any `dsp!`, are evaluated HERE, before step 2.]

1b. SYNC ROUTE (bit 7 of ctl, predicted not-taken)
   ctl & 0x80 ⇒ format on this thread, write_oracle_line() (BOUNDED — D9c),
   TARGET_STATS.sync_routed += 1, return.  ~200+ ns, per-target opt-in (D20).

2. RATE (Warn/Error only)
   Once/OnceCounted && FIRED.load(Relaxed) ⇒ [OnceCounted: suppressed RMW] return
                                             // per-SITE static, private line (F11)
   EveryN(n)  ⇒ (count.fetch_add(1) & (n-1)) != 0 ⇒ return   // n is pow2 (X3)
   MinInterval⇒ policy RMW
   Every      ⇒ skip
   Downstream code: idx = idx_cell.load(Relaxed); UNASSIGNED ⇒ #[cold] mint (D19)

3. SAMPLE (k = (ctl >> 3) & 0x0F; predicted not-taken when k == 0)
   c = SAMPLE_CTR[lane][target].get().wrapping_add(1)
   SAMPLE_CTR[lane][target].set(c)                    // plain load/store, NO RMW
   (c & ((1<<k)-1)) != 0 ⇒ sampled_out.fetch_add(1, Relaxed); return

4. LANE  = TLS `MY_LANE`. Cold miss ⇒ claim (§B). Claim failure ⇒
   Warn/Error: write_oracle_line() (synchronous fallback, M26); else
   UNLANED_DROPPED.fetch_add(1, Relaxed); return.

5. SIZE   need = HEADER_BYTES + args.encoded_len()          // runtime (B2)
   need > MAX_RECORD_BYTES ⇒ drop + TOO_LARGE count; return  // N29, release-live

6. SPACE + WRAP  (records never straddle — B3; arithmetic corrected — F6)
   w    = write.load(Relaxed);  off = w & MASK
   tail = LANE_BYTES - off
   // Rule shared VERBATIM by producer and consumer:
   if tail < HEADER_BYTES { pad = tail }                     // implicit wrap
   else if tail < need    { pad = tail }                     // explicit PAD record
   else                   { pad = 0 }

   // ── admission control. NO unsigned subtraction here can go negative. ──
   //   CAPACITY = LANE_BYTES - 1        (one slot reserved: full vs empty)
   //   INVARIANT used <= CAPACITY, inductive over the producer's own
   //   admissions: read_cached <= read <= w always, and the producer only
   //   publishes w + pad + need after proving pad + need <= avail.
   used   = w.wrapping_sub(read_cached)
   debug_assert!(used <= CAPACITY);
   avail  = CAPACITY - used                                  // cannot underflow
   budget = if level == Error { avail } else { avail.saturating_sub(ERROR_RESERVE) }
   //       ^ SATURATING is the fix. v2 computed `LANE_BYTES - ERROR_RESERVE
   //         - used`, which underflowed to ~4.29e9 in exactly the state the
   //         reserve exists to create, and licensed an overrun of live bytes.
   if budget < pad + need {
       read_cached = read.load(Acquire); recompute used/avail/budget;
       if still short {
           if dropped.load(Relaxed) != u32::MAX {             // saturation (D5, D21)
               dropped.fetch_add(1, Relaxed);
               dropped_bytes saturating-add need;
           }
           return
       }
   }
   if pad >= HEADER_BYTES { write PAD header (site = null, len = pad) at off }
   w = w.wrapping_add(pad); off = w & MASK   // now off == 0 or tail >= need

7. WRITE   write_unaligned(off, RecordHeader{ site, tsc: rdtsc(), len: need, flags })
           args.encode(off + HEADER_BYTES)
8. PUBLISH write.store(w.wrapping_add(need), Release)
```

- **Complexity** O(1); O(len) memcpy for inline `&str`.
- **Cache** strictly sequential streaming writes into the ring tail. Working set: the `CONTROL` line, the producer line, the lane's `SAMPLE_CTR` row segment (one line), 1-2 ring-tail lines — **≤ 4 lines, unchanged from v2**, because the sampling row is the only addition and it is one line and producer-private. A `Once` site's `FIRED` static is a fifth line only on the `Warn`/`Error` path, which is not the budgeted path. `LANES`, `CONTROL` and `SAMPLE_CTR` have compile-time-known addresses — no pointer chase.
- **Branching** 3 (or 2, dynamic) predicted-not-taken gates + sync + rate + sample + wrap + space. `budget` is a `saturating_sub`, i.e. `sub` + `cmov` — still branchless. The sync and sample branches are not-taken in every default configuration, so I-cache pressure is one extra `cmp/jcc` pair each.
- **Inlining** steps 1-3 `#[inline]` (must fold). Steps 4-8 in `#[inline(never)] fn emit_impl<A: LogArgs>` — monomorphised per argument-tuple type. Blanket `#[inline(always)]` would replicate ~60 instructions at every site and bloat L1i, which principle 7 forbids on measurement grounds.
- **SIMD** none wanted: the payload is ≤ 2 KiB and moves by `copy_nonoverlapping`, which already lowers to the best available move sequence. There is no vectorisable reduction anywhere in `boyko_log`.

### B. Lane claim / retire

```
CLAIM (cold, once per thread):
  // Written over an ITERATOR, not `LANES[i]`, so LANE_ARRAY_LEN == 0 in the
  // `off` build is zero iterations rather than dead-but-panicking code (F21).
  for (i, lane) in LANES.iter().enumerate().cycle_from(spread(thread_id)):
    if lane.owner.load(Relaxed) != FREE { continue }             // load first (M10)
    if lane.owner.compare_exchange(FREE, token, Acquire, Relaxed).is_ok() {
        seed SAMPLE_CTR[i][t] = (i * 0x9E37) as u16 for all t    // phase break (D20)
        MY_LANE.set(i); install TLS guard; return i }
  ⇒ exhausted: MY_LANE.set(NONE)   // step 4 above handles the fallback

RETIRE (TLS guard Drop, producer thread, after its last write):
  owner.store(RETIRING, Release)

RECLAIM (consumer, per drain, after staging):
  if owner.load(Acquire) == RETIRING && read == write { owner.store(FREE, Release) }
```

### C. Drain — staged copy BEFORE the free *(fixes B1)*

```
STAGE_BYTES = 256 KiB (a `.bss` static, sink-owned, reused every drain — F25)

drop_tally = {lanes: 0, records: 0, bytes: 0, saturated: false}
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
  fold this lane's dropped / dropped_bytes / sampled_out into drop_tally,
      TARGET_STATS and LogStats; clear with fetch_sub(observed) unless SATURATED
  reclaim if RETIRING && r == w
if drop_tally.records > 0 { synthesise ONE boyko-W0102 for the whole drain }
      // one per DRAIN, not one per lane: 125/s instead of ~16 000/s (F24)
sort_unstable_by_key(tsc)         // over preallocated 16 B triples; no allocation
for each staged record, for each ACTIVE sink whose filter accepts (target, level):
    text sinks : (*site.decode)(stage + s + HEADER_BYTES, len, &mut fmt)  // reads STAGING
    binary sink: site_id = SITE_DICT lookup (#[cold] miss ⇒ dictionary record)
                 append {site_id, tsc_delta, len, flags, payload}  // NO formatting (D22)
one write_all per byte sink per fill; push formatted lines to the ECS handoff ring
```

**Fan-out is inside ONE drain.** Every sink reads the same staging arena; there is never a second consumer of a lane. That is what makes "text + binary + crash simultaneously" cost one pass, and it is why §Refused rejects a second sink thread.

**Why the order changed.** v1 advanced `read` at step 3 and decoded at step 6 from an `offset` **into the ring** — bytes the producer was licensed to overwrite in between. The sink would then read a torn header and call `decode` through 8 arbitrary bytes reinterpreted as a function pointer. v1's tests could not see it: both the ordering test and the overflow test drive a quiesced producer. The staged copy makes the window structurally absent, and adds a bound: a drain never stages more than `STAGE_BYTES`, so a hot lane is drained across several passes rather than in one unbounded burst.

**Provenance.** The header — the only field carrying a pointer — is moved by a **typed** `read_unaligned`/`write` pair, never by a byte memcpy, so `site`'s provenance round-trips by construction rather than by relying on per-byte provenance tracking. Payloads are pointer-free POD and move by `copy_nonoverlapping`. Gated by Miri under Tree Borrows (test 14).

- **Complexity** O(R log R) per drain for R records, entirely off the frame thread.
- **Stated limitation** cross-lane ordering is *approximate*: a record written after lane A's snapshot may carry an earlier `tsc` than one already staged from lane B. Inherent to any non-blocking merge (Quill has the same property) and printed in the sink's header line.

### D. `flush`

```
1. match SINK_STATE.load(Acquire) {
       NotBooted | Manual | Exited | CrashDraining => return FlushResult::NoConsumer,
       Running | Exiting => {}
   }
2. epoch = FLUSH_REQ.fetch_add(1, AcqRel) + 1
3. unpark(sink)
4. spin_backoff until FLUSH_ACK.load(Acquire) >= epoch, deadline = now + 2 s
5. on timeout: write_oracle_line("boyko-E0105: log flush timed out"); TimedOut
   // write_oracle_line is BOUNDED (≤ 50 ms, then steal) — Decision 9c. v2's
   // bounded wait terminated in an UNBOUNDED one (F8).
```
Step 1 is what keeps `#[should_panic]` tests in an unbooted binary at zero cost. Step 5 is non-negotiable: the profiling audit's central finding is that an unbounded blocking wait converts an instrumentation gap into an unkillable hang, and that this repository has no kill-after-timeout pattern to borrow (`vb_bench_totality_gate.rs:48-49`). This design does not add a second one.

### E. Crash drain *(Decision 24)*

```
panic hook (chained ahead of the existing hook):
1. write_oracle_line(panic message)                       // synchronous, bounded, always
2. match flush() { Flushed => return, _ => {} }
3. for from in [Exited, NotBooted, Manual]:
       if SINK_STATE.compare_exchange(from, CrashDraining, AcqRel, Relaxed).is_ok() {
           // The consumer role is now PROVABLY exclusive: each of these three
           // states means no sink thread can be inside a drain.
           run Algorithms C over every lane, text sink = CRASH sink only
           write_oracle_line("boyko-E0109: crash drain took the consumer role")
           SINK_STATE.store(Exited, Release); return
       }
4. // SINK_STATE == Running or Exiting: the sink exists and may be inside a
   // drain. DO NOT displace it — two consumers on one lane is what the Lane
   // SAFETY block forbids. flush() already timed out and said so (E0105).
   return
```
- **Termination**: step 3 is a bounded loop over three constants; step 4 returns. No wait is added.
- **What it cannot do**: survive `abort()`, `SIGSEGV`, or a guard-page stack overflow — the hook does not run. Written down in E22 and in G14's "cannot claim" column rather than mitigated by a heuristic.

---

## Multithreading model

| Datum | Sharing | Ordering | Why |
|---|---|---|---|
| `Lane::buf` | SPSC | none (guarded by `write`) | payload published by the cursor's Release |
| `Lane::write` | P→C | `Release` / `Acquire` | the happens-before edge for the payload |
| `Lane::read` | C→P | `Release` (after staging) / `Acquire` | frees space only once bytes are copied out (B1) |
| `read_cached` / `write_cached` | private `Cell`, single-role | none | the half that actually buys throughput; SAFETY clauses 1e/1f cover them (F23) |
| `Lane::owner` | MPMC (claim) | `load` then CAS `Acquire`; `Release` on retire/free | contended once per thread lifetime |
| `dropped`, `dropped_bytes`, `sampled_out` | P adds, C subtracts | `Relaxed` | own cache line; `fetch_sub(observed)` never loses a concurrent add; saturation is a load-then-add, still `Relaxed` |
| `CONTROL[i]` | MP-read, rare CAS write | `Relaxed` read, `AcqRel` CAS | a stale ceiling for one record is documented as acceptable; the CAS preserves sibling bit-fields (D14) |
| `CONTROL_EPOCH` | 1W-ish / MR | `Release` add / `Acquire` load | derived and monotone; carries no state, so it cannot diverge from `CONTROL` |
| `SAMPLE_CTR[lane][t]` | **single writer** = lane owner | `Relaxed` load + store, **never an RMW** | the row index IS the lane index ⇒ SAFETY clause 1 applies verbatim; no sharing, no lock prefix |
| per-site `FIRED` (`Once`) | MP | `Relaxed` load; one lifetime `swap` | steady state is a pure load from a **site-private** line (M11, F11) |
| `RATE[idx]` (`EveryN`/`MinInterval` only) | MP | `Relaxed` RMW | opt-in; cost documented at the declaration site |
| `CodeIdx::Dynamic` cell | MP | CAS `UNASSIGNED→RESERVED`, `fetch_add`, `store(Release)`; readers `Acquire` | reserve-then-publish: dense, no leaked indices, one CAS per code per process (D19) |
| `DYN_NAMES[i].hash` | MP insert-only | bytes+len stored, then `hash.store(Release)`; readers `Acquire` | a reader seeing a hash sees a complete name |
| `TARGET_STATS[i]` | C writes, MR reads | `Relaxed` | one writer (the consumer-role holder); readers tolerate one-drain staleness, which the census states |
| `SINKS[n].state` / `.filter` / `.floor` | MW stores, C reads | `Relaxed` | policy only; acting on a one-drain-stale filter is a documented property (G13), not a race |
| `SINK_REQ` | MP producers, 1 consumer | `AcqRel` seq; writes under `OUT_LOCK` | cold, human-initiated, bounded at 16; full ⇒ `E0107` |
| `FLUSH_REQ` / `FLUSH_ACK` / `SINK_STATE` / `SINK_EXITED` | 2-way | `AcqRel` / `Acquire` | completion must be observed, not guessed; `SINK_STATE`'s CAS is what proves crash-drain exclusivity (D24) |
| `OUT_OWNER` (`OUT_LOCK`) | MP | CAS `Acquire` / `Release`; **bounded acquire, RAII release, re-entrancy detected** | `report!`, the exhaustion fallback, sync-routed targets, `SINK_REQ` writes. Protocol in Decision 9c; the Vulkan messenger is **not** a caller (D9b) |
| Sink array kinds, `LogConfig` | boot-published | one `Release` at boot | kinds never mutate after boot; only state/filter/floor do |

**Data-race freedom.** No lane has two producer *threads* (CAS from FREE confers exclusive write rights). No lane has two producers *re-entrantly* on one thread (no user code runs inside the open window — Decisions 13 and 19b, `debug_assert`ed and asserted by test 24). **No lane ever has two consumers**: the sink thread holds the consumer role for the whole of `SINK_STATE == Running`, `Manual` asserts single-call, and the crash path acquires the role only by a CAS out of a state in which no sink thread can be executing drain code (Decision 24). `SAMPLE_CTR` rows inherit the single-writer property from the lane index. Payload visibility rests on the `Release`/`Acquire` cursor pair, and the consumer never reads past its observed `w` **nor advances `read` over bytes it has not staged**. Reclaim is ordered by `RETIRING` being stored after the producer's last write and observed only after the consumer has drained to `write`.

**`Send`/`Sync`.** `Lane: Sync` via the documented manual impl; `DynSlot`, `TargetStatCell`, `SinkSlot`: `Sync` via impls whose SAFETY blocks name the single-writer or atomic-only argument. `TargetId`, `TargetControl`, `WarnCode`, `ErrorCode`, `PanicCode`, `Level`, `SessionId`: `Copy + Send + Sync`. `LogRing`, `LogStats`, `LogCensus`: `Send + Sync`, ordinary `Resource`s. **No `!Send` handle exists** (Decision 12). `LogPod: Send + Sync` is required so a game type cannot smuggle a thread-affine value onto the sink thread.

---

## What this system can and cannot substitute for — the sync-validation confrontation

The audit established, from source, that `is_instance_extension_present(global, VK_EXT_VALIDATION_FEATURES_EXTENSION_NAME)` at `crates/boyko_rhi_vulkan/src/device.rs:2110` queries `vkEnumerateInstanceExtensionProperties` with `pLayerName == NULL`, which returns the implementation's own extensions plus implicitly-enabled layers' — never those of an explicitly-requested layer. `VK_EXT_validation_features` is supplied by `VK_LAYER_KHRONOS_validation`. Therefore `sync_validation_available` is always false, the `VkValidationFeaturesEXT` node is never chained, and `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT` is never requested. This matches the measured fact that a genuine missed barrier produced **19 messages (= baseline), zero `SYNC-HAZARD`, and a byte-identical golden**.

**A logger is a transport. It changes where a message goes and has no opinion on whether the message exists.** Therefore, explicitly:

1. **Routing validation output through `boyko_log` does not make a missed barrier visible.** Not one hazard becomes detectable. Any sentence of the form "the validation layer would have told us" remains false on this machine, before and after this plan.
2. **It would make the deadness easier to miss** — a clean, colour-coded log reads as evidence of a clean run. **And it would make the evidence droppable.** That is why v1's migration is withdrawn (Decision 9b): the channel stays synchronous, and the only change is deleting an allocation.
3. **The logger's legitimate contribution is to make ABSENCE loud.** Two mandatory mechanisms:
   - **`boyko-E2101`** — emitted at boot when validation is *requested* but the features node was not chained. A liveness claim about the channel, not about the frame.
     **Its two-sidedness is re-cut, because v2's negative leg was unbuildable here** *(fixes F2)*. v2 required a fixture "where the node is chained". By this document's own proof, `sync_validation_available` is **always false on this machine**, so that fixture cannot exist without the RHI fix the plan excludes (open question 6) — M25's disposition installed a second control this machine cannot show, which is the exact defect M25 was raised about. v3's G7 is two-sided over a predicate that **can** be driven both ways here: **(a)** a validation-**on** run ⇒ `E2101` fires; **(b)** a validation-**off** run (`BOYKO_DISABLE_VALIDATION=1`, the machine's documented switch) ⇒ `E2101` is **absent**. Both legs run today. **What G7 cannot claim**: anything about the chained case. It proves the code reports the gap when validation is requested and stays quiet when it is not; it does **not** prove the node would be chained if the extension were found. That claim belongs to the RHI fix, and `E2101` exists precisely to keep the gap greppable until then.
   - **The `LOG-CENSUS`** — at every `flush()` and at shutdown, one line per armed target: `LOG-CENSUS target=vk-validation level=Warn records=0 dropped=0 status=UNPROVEN`. A target that has never delivered a record is `UNPROVEN`, never `clean`. The status vocabulary now covers **five** ways to manufacture a silence, three of them created by the game-facing extension itself: `UNPROVEN(lossy)` (`dropped > 0`), `UNPROVEN(sampled)` (a non-zero shift — the count is `1/2^k` of the truth), `UNPROVEN(unsunk)` (**no `Active` sink's filter accepts this target** — a game enables a category, sees nothing, concludes clean), and `dropped=SATURATED(...)` (the counter hit its ceiling and is no longer a number). This is the direct translation of "a gate that cannot fail is a defect" into the logging system, extended to the new ways a game can build one.
4. **The underlying fix is out of scope and belongs to the RHI** (`pLayerName = "VK_LAYER_KHRONOS_validation"`). This plan does not fix it, does not claim to, and adds `E2101` so the gap has a coded, documented, greppable identity until it is fixed.
5. **What no logger can substitute for:** an absent source. Sync-validation, the layer's `INFO`/`VERBOSE` severities (structurally excluded at `debug.rs:125-126`), GPU pipeline statistics (hard-wired to `0` at `rhi_impl/device.rs:1013`), and per-system CPU timing (does not exist) are four separate named gaps, and none is closed by this plan.

---

## Integration

### New
- **Crate `boyko_log`** — `level.rs`, `control.rs`, `target.rs`, `site.rs`, `record.rs`, `lane.rs`, `codes.rs` (generated), `rate.rs`, `sample.rs`, `sync_out.rs` (`OUT_LOCK`, `report!`, `write_oracle_line`), `sink/{mod,console,file,binary,callback,crash,ecs,request}.rs`, `macros.rs`, `tsc.rs`, `session.rs`, `bin/logdec.rs`, `build.rs`.
- **`docs/diagnostics/<code>.md`** — one per code; check 2's target; published by `doc-writer`.
- **`crates/boyko_ecs/src/ecs/core/log/`** — `LogPlugin`, `LogRing`, `LogStats`, `LogCensus`, `log_drain_system`.
- **`crates/boyko_macros`** — `#[derive(LogPod)]` (no new dependency edge: `boyko_macros` is a proc-macro crate with no `boyko_ecs` dependency).
- **`crates/boyko_log/tests/`** — `code_registry.rs` (the eight checks), `print_census.rs` (the tidy-style print ban, sharing the walker), `untested_codes.txt` and `print_allowlist.txt` (data files, each excluded from its own scan).
- **`docs/LOG-BINARY-FORMAT.md`** — the `.blog` schema with `schema_version`; the decoder refuses a mismatch.
- **`docs/HOT-PATH-EXCEPTIONS.md`** — **NO new entry.** See Invariant 1: a row for `OUT_LOCK` reds `scripts/check_hotpath_exceptions.py` because the file carries no `#[allow(clippy::disallowed_types)]` for it to match (F9).

### Migration ledger — machine-generated, not hand-tabled *(fixes M22)*
v1's table covered ~14 files against a measured **179 occurrences across 36 files** under `crates/**/src/**` (this session's grep). The migration is driven by a generated ledger, `docs/diagnostics/PRINT-CENSUS.md`, regenerated by the same walker that backs the enforcement test, with every site classified into exactly one of:

| Class | Count (measured) | Disposition |
|---|---|---|
| CLI binary stdout (`boyko_shaderdsl/src/bin/*`) | 58 | **Keep.** One crate-level `#![allow]` + rationale per bin, not per site. |
| In-`src` `#[cfg(test)]` modules | 23 (`sdf_math/brick/tests.rs` 3, `rhi_vulkan/compute/tests.rs` 16, `physics/solver/colored_tests.rs` 4) | **Keep.** Excluded by the walker's `#[cfg(test)]`-region rule. |
| Measurement lines (`runner.rs` ~20) | 20 | → **`report!`**, byte-frozen (Decision 9). |
| Validation messenger (`debug.rs:114`) | 1 | **UNTOUCHED** (Decision 9b, F12). Allowlisted with a reason, allowlist checked both ways. |
| Prose mentions inside comments (e.g. `runner.rs:560-561`) | ≥ 2 | **Not sites.** The walker's CODE stream never sees them, so they are not in the denominator (F18). |
| Everything else (production diagnostics) | the remainder of the 98 | → `error!`/`warn!`/`info!` with codes. |

**The denominator, restated**: 179 raw occurrences − 58 − 23 = **98** occurrences to disposition (v2 said 83 — arithmetic error, F18), of which the walker's CODE stream will resolve some number ≤ 98 as actual macro invocations. L8c's exit criterion is `Pending` == 0 in the registry (check 3c) plus zero unclassified walker sites — two integers, both machine-produced.

Named production files v1 omitted and that the ledger covers: `rhi_vulkan/present/targets.rs` (7), `render/texture.rs` (7), `app/{host_dump,hzb_dump,vg_census_dump,vb_probe_dump,vb_cull_probe}.rs` (14), `app/plugins.rs` (3), `app/gpu_scene/mod.rs` (3), `app/host.rs` (2), `physics/soft/self_collision.rs` (3), `ui/layout.rs` (2), `rhi/handle.rs` (2), `serialize/load.rs` (2), `ecs/asset/server.rs` (1), `ecs/ecs_master/system_api.rs` (1), `image/{png,inflate}.rs` (2), `render/{bindless,mesh_geometry_table,light_system,render_path_config,gpu_system}.rs` (8), `threadpool/worker.rs` (1), `ecs/schedule/schedule_builder.rs` (2).

### Behaviour changes worth naming

| Site | Change |
|---|---|
| `boyko_threadpool/src/worker.rs:157-168` | `abort_on_task_panic` → `error!(codes::E0201, …)` + **`flush()` before `abort()`** |
| `boyko_ecs/.../schedule_builder.rs:1334-1350` | `warn_if_empty` → `warn!(Schedule, codes::W1501, …)`; text normalised (substring-safe) |
| `boyko_ecs/.../params/diagnostics.rs:53` | `error[boyko-B0002]:` → `boyko-B0002:` (substring-safe) |
| `boyko_ecs/.../events/event_buffer.rs` | overflow emits `warn!(codes::W0701, type_name, lane, attempted, dropped)`; the `Result` is unchanged. Those four fields currently exist only inside an `EcsError` nobody reads |
| `boyko_ecs/.../query_type_registry.rs:124-144` | `warn!(codes::W0501)` at 75 % occupancy; the terminal `panic!` gains `B0502`. 1023 silent mints then a process kill is not a diagnostic |
| `boyko_rhi_vulkan/src/debug.rs:114` | **NOT TOUCHED AT ALL** (Decision 9b, F12). v2's `to_string_lossy()` removal is withdrawn: `Cow::Borrowed` means there is no allocation to remove on the normal path, and writing raw `CStr` bytes would change the emitted bytes in the invalid-UTF-8 case on a byte-frozen gate-oracle channel pinned at `boyko_app/tests/vb_bench_query_validation.rs:116-118`. Added to `print_allowlist.txt` with that reason |
| `boyko_rhi_vulkan/src/device.rs:2110` | add `error!(codes::E2101)` when validation is requested but the node was not chained |
| `boyko_rhi_vulkan/src/device.rs:3100,3158,3189` | drop `#[cfg(debug_assertions)]` → `warn!(codes::W2102)`, `RatePolicy::Once`. **Because `Once` is now per-SITE (Decision 8, F11), all three degradations report** — a code-scoped `Once` would have printed one and silently lost two. Settles the two-doctrine conflict in favour of `boyko_app/src/host.rs:228-233`'s written argument that a release-build degrade-to-disabled must be observable |
| `boyko_render/src/render_path_config.rs:311-337` | delete both hand-rolled latches (the per-frame `swap` bug) → `warn!` + `Once` |
| `boyko_render/src/light_system.rs:397,456` | delete latches → `warn!(codes::W2201, dropped_count)`; **the dropped count is now reported**, which the one-shot latch never did |
| `boyko_render/src/{bindless,mesh_geometry_table}.rs` | ad-hoc `"WARN: "` → `warn!(codes::W2202)`; keep `debug_assert!(false)`. Same per-site `Once` argument as `W2102` |
| `boyko_app` boot | selects a `LogConfig` profile (Decision 25) and generates the `SessionId`; `boyko_demo` gains a console command bound to `apply_control_spec` as the worked example of runtime control |
| `boyko_render/src/gpu_system.rs:399-404` | → `error!(codes::E2203)`. The `System` trait's missing error channel stops mattering: the logger is a side channel available from any thread |
| `boyko_image/src/{png.rs:206, inflate.rs:656}` | → `warn!(codes::W2601/W2602)`; decoding continues |
| `boyko_app/src/runner.rs` (20 measurement sites) | → `report!`, text unchanged |

### Enforcement *(fixes M23)*
**Primary: an in-repo tidy-style test**, `crates/boyko_log/tests/print_census.rs`, which walks `crates/*/src/**.rs`, excludes `src/bin/` and `#[cfg(test)]` regions, asserts a non-empty corpus, and fails on any `println!`/`eprintln!`/`print!`/`eprint!` outside `tests/print_allowlist.txt` — with the allowlist checked in **both** directions. We own it, and it can be shown red in one line.

**Secondary: `clippy.toml`'s `disallowed-macros`**, added only after a **shown-red canary**: `clippy.toml:21-25` records, empirically, that clippy *silently ignores a config path it cannot resolve*. The L8 gate compiles a deliberate `println!` and records the observed diagnostic in the plan's own gate log; if the key is inert on the pinned clippy, the entry is dropped and the tidy test stands alone. Independently noted: the lint cannot see `stdout().write_all`, `io::Write` on a raw handle, or `libc::write`, so it could never have carried the migration claim by itself.

### Compatibility
`Arena` / `ComponentPool` / `UnitId` untouched. `LogRing` and `LogCensus` use `VmReservation`-backed columns whose element sizes divide `COMMIT_GRANULE`, pinned by const asserts (M13, F7). `golden.ps1:226`'s `[vk-validation]` grep: preserved, still synchronous, **and its producer is not edited at all** (F12). `vg_occ_split_timing.rs`'s `VB-P1d` parse: preserved; note that this consumer reads `out.stdout` and `out.stderr` as **separate** buffers (`:1115-1117`), so it was never exposed to cross-stream interleaving — the real ordering fix is `report!` writing through `stdout()`'s own `LineWriter` rather than a raw handle (F13, F17). **No v3 feature writes to stdout**, so no golden and no parser moves.

---

## Implementation plan

Each rung is independently green (`cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace` — **`--workspace` is mandatory**: a root `cargo check --all-targets` is vacuously green here because of the virtual-manifest quirk) and commits alone. L0-L9 are v2's ladder with the fixes folded; L10-L17 are the scope extension, each purely additive to the rung before it.

| # | What | Where | Must NOT move |
|---|---|---|---|
| **L0** | Skeleton; `Level`, `LogTarget`, `TargetId` (private field), `TargetControl` (**packed byte**), `targets!` table, `CONTROL`, `CONTROL_EPOCH`, the five macros with the 3-gate expansion, `GLOBAL_CEILING` const + `build.rs`. **No sink, no `emit_impl`.** | `src/{level,control,target,macros}.rs` | nothing exists yet |
| **L0-gate** | **G4** three-way side-effect probe: (a) compile-ceiling-below with runtime armed to Trace ⇒ 0; (b) both armed ⇒ **1000**; (c) runtime ceiling `Off` ⇒ 0; **(d) shift = 1 ⇒ exactly 500**. Debug *and* release. **G2** separate build leg `BOYKO_LOG_MAX_LEVEL=off`, all three legs with their mechanisms (Decision 3). **G1 is NOT here** — it moves to L1-gate (F19): L0 has no `emit_impl`, so G1's positive leg could not run and the gate would degrade to "the disabled fixture lacks a symbol nothing has". | `tests/gates_disabled.rs`, CI leg | — |
| **L1** | `Lane` (3 partitions + `sampled_out`), layout asserts, load-then-CAS claim, retire, wrap protocol, **the corrected admission arithmetic (F6)**, `LogValue`/`LogArgs`, `dsp!`, saturating drop counting, `MAX_RECORD_BYTES` runtime check, `emit_impl`. | `src/{lane,record,site}.rs` | L0's gate expansion |
| **L1-gate** | **G1** symbol gate (now that `emit_impl` exists). **G17** Error-reserve arithmetic, two-sided. Per-thread zero-alloc gate incl. the first-emit TLS allowance (F26); loom model of claim/retire + cursor pair; wrap-boundary proptest; **cursor-wrap-at-2³² test (E17)**; overflow test asserting `dropped > 0`; **G3** `.bss` section gate; **G5** distinct-`decode`-symbol upper bound (N31). | `tests/`, `scripts/section_gate.ps1` | — |
| **L2** | `codes!` (with `prefix`, `status`), `DiagInfo`, dense `code_idx`, the three code newtypes, power-of-two `EveryN` assert, per-site `Once` latch expansion, `docs/diagnostics/` seeded with the 9 grandfathered codes as **`Pending`** + the `B9003` gap note, `explain()`. | `src/codes.rs`, `docs/diagnostics/` | code numbers |
| **L2-gate** | The **eight** registry checks (integration test) over the specified three-stream walker. Check 3c stays disarmed until L8c. Each check **shown red once** against a deliberately broken registry; the observed failure text recorded in the gate log. **This rung commits alone because the grandfathered codes are `Pending` and check 3b requires them to have no emitters yet (F20).** | `tests/code_registry.rs` | — |
| **L3** | `sync_out.rs` (`OUT_LOCK` per Decision 9c, `report!` through `stdout()`'s `LineWriter`, `write_oracle_line`), `tsc.rs`, `session.rs`, sink thread with adaptive park, staged drain, console sink → stderr, `flush`/`shutdown` with `SINK_STATE`, panic-hook chaining. | `src/{sync_out,tsc,session,sink/*}.rs` | `report!` text |
| **L3-gate** | **G18** `OUT_LOCK` two-sided (unwind release; re-entrant completion). Flush-without-consumer ⇒ `NoConsumer` immediately; flush-timeout ⇒ within 2 s; shutdown detaches; **`sink_sustained_rate`** finds the drop knee (M19); **M24** concurrency test — sink flooding while `report!` prints, `golden.ps1`'s merged-stream scan must still resolve. | `tests/`, `benches/` | — |
| **L4** | File sink + cap (`W0103`), rate limiter, `LOG-CENSUS` incl. `UNPROVEN(lossy)`, `SinkMode::Manual`. | `src/{sink/file,rate}.rs` | — |
| **L4-gate** | `Once` steady state performs **no store** (assembly/`perf` check) **and touches no shared line** (per-site latch, F11); census `UNPROVEN` at 0 records **and** at `dropped > 0`. | `tests/`, `benches/` | — |
| **L5** | ECS seam: `LogPlugin`, `LogRing` (16 B `LogLine`) on `VmReservation`, `LogStats`, `log_drain_system`. | `crates/boyko_ecs/.../log/` | the `COMMIT_GRANULE` divisibility asserts |
| **L5-gate** | **P1, re-specified** (F3): the perturbation instrument is a **headless schedule bench**, not windowed frame time. ABBA-counterbalanced, interleaved zero control, same sitting. | `crates/bench_bevy_vs_boyko/benches/` | — |
| **L6** | Migrate `boyko_ecs` + `boyko_threadpool`; flip those rows `Pending`→`Live`; `W1501`, `B0002` normalisation, `W0701`, `W0501`/`B0502`, `E0201`. | as tabled | `#[should_panic]` substrings |
| **L7** | Migrate `boyko_rhi_vulkan` **except the messenger, which is not touched at all**; `E2101`; `W2102` ungated in release; census wiring. | as tabled | `[vk-validation]` line, byte for byte |
| **L7-gate** | **G7, re-cut two-sided** (F2): `E2101` fires on a validation-**on** run and is absent on a validation-**off** run (`BOYKO_DISABLE_VALIDATION=1`). Channel liveness is proved separately by an **ordinary validation error from a deliberately invalid call** — the historical `mip_levels: 12` on a 512×512 image — with the **baseline of 19 messages accounted for**. A forced *hazard* is explicitly **not** the control: this machine has been measured unable to produce `SYNC-HAZARD` (M25). | `crates/boyko_rhi_vulkan/tests/` | — |
| **L8a** | Migrate `boyko_render`, `boyko_image`, `boyko_serialize`, `boyko_physics`. | ledger | goldens |
| **L8b** | Migrate `boyko_app`; measurement lines → `report!`. | ledger | `VB-P1d`/`VB-P4` text |
| **L8c** | Check 3c armed: `Pending` == 0. Walker's unclassified-site count == 0; enable `print_census.rs`; run the clippy `disallowed-macros` canary and record the result. | `tests/`, `clippy.toml` | — |
| **L9** | `boyko_ui` console widget over `LogRing`. **Deferred to the UI plan** — L16 fixes the whole contract it consumes, so nothing logging-shaped remains in it (open question 12). | `crates/boyko_ui/` | — |
| **L10** | **Dynamic targets.** `DYN_NAMES` interning, `register_dynamic_target`, `find_target`, `targets()`, the five `dyn_*!` macros, `E0106`. | `src/target.rs`, `src/macros.rs` | static-target expansion byte-for-byte; G1/G4 must still pass unchanged |
| **L10-gate** | **G8** (a-d). Bench `log_dyn_disabled` vs `log_disabled_runtime`. | `tests/`, `benches/` | — |
| **L11a** | **Downstream code tables.** `codes!` exported with `prefix`; `CodeIdx::Dynamic` + lazy minting; `codes_tidy!`; `CODE_OCCUPANCY` + `W0114`. | `src/codes.rs` | engine `code_idx` remains a compile-time constant |
| **L11b** | **`LogPod`** + `#[derive(LogPod)]` + the `*_kv!` field-name forms. | `boyko_macros`, `src/site.rs` | Decision 13's structural property (asserted by test 24) |
| **L11-gate** | **G9**, **G9b**. | `tests/` | — |
| **L12** | **Sampling.** `SAMPLE_CTR`, the claim-time seed, step 3 of Algorithms A, `sampled_out` plumbing, `W0113`, census `UNPROVEN(sampled)`. | `src/sample.rs`, `src/lane.rs` | the ≤ 15 ns enabled target — **G10d decides whether this rung ships default-on** |
| **L12-gate** | **G10** (a-d), including the perturbation control that can flip `log-sampling` to default-off. | `tests/`, `benches/` | — |
| **L13a** | **Volume, part 1.** `Rotation`, `W0112`, saturating counters end-to-end, `LogStats` u64 accumulation, `LogRing` cursor-wrap hardening, census `SATURATED`. | `src/sink/file.rs`, `src/lane.rs`, ECS seam | `Rotation::NONE` remains the engine default |
| **L13a-gate** | **G11** two-sided. | `tests/` | — |
| **L13b** | **Volume, part 2.** `BinarySink`, `SITE_DICT`, `SINK_OUT`, dictionary records, `logdec`, `docs/LOG-BINARY-FORMAT.md`. | `src/sink/binary.rs`, `src/bin/logdec.rs` | text-sink output byte-for-byte |
| **L13b-gate** | **G12** (a-c) — **including the revert clause**. | `tests/`, `benches/` | — |
| **L14** | **Runtime sink control.** `SinkSlot` state/filter/floor, `SINK_REQ`, `request_open_file`/`request_close`, `E0107`, `ControlSource::File` + `apply_control_spec`, census `UNPROVEN(unsunk)` + `W0111`. | `src/sink/request.rs`, `src/control.rs` | no I/O on a caller thread |
| **L14-gate** | **G13** (a-c). | `tests/` | — |
| **L15** | **Crash path.** `CrashSink` opened at boot, `SINK_STATE::{Exiting, CrashDraining}`, the panic-hook protocol, `E0109`. | `src/sink/crash.rs`, `src/sink/mod.rs` | Decision 12's flush semantics; no new unbounded wait |
| **L15-gate** | **G14**, two-sided. | `tests/` | — |
| **L16** | **Game consumption.** `TARGET_STATS`, `LogCensus`, `LogRing::since` + `RingFilter` + `skipped`, the per-frame `EPOCH` record, `SessionId` in every header. | `src/target.rs`, `crates/boyko_ecs/.../log/` | the drain stays off the frame thread |
| **L16-gate** | **G15**, two-sided. | `crates/boyko_app/tests/` | — |
| **L17** | **Shipping profiles.** The four `LogConfig` presets, profile name in every header, CI legs for `dev` / `shipping` / `shipping-min` / `off`. | `crates/boyko_app/src/`, CI | G2's `off` leg must still pass unchanged |
| **L17-gate** | **G16** two-sided symbol gate + **P2** soak. | CI legs, `tests/` | — |

Ordering constraints: L10 before L11a (a dynamic target is the first consumer of a downstream code); L12 after L1; L13b after L13a (rotation is shared); L15 after L13a (the crash sink shares the file machinery); L16 after L12 and L13a (`TargetStat` carries `sampled_out` and the saturation flag); L17 last.

---

## Metrics and validation

### Benchmarks (`crates/boyko_log/benches/emit.rs`, criterion, `harness = false`)
Every row runs against a control **in the same sitting** — because this repository has measured its own wall-clock floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol, a number without an in-sitting control is not a measurement. No benchmark binary may contain `time` / `update` / `setup` / `install` / `patch` in its name (Windows os-error-740). Never two bench jobs concurrently (`target/` once reached 74 GB and took the disk to zero, masquerading as mingw errors).

| Bench | Target | Control |
|---|---|---|
| `log_disabled_runtime` | ≤ 3 ns | the same site enabled; **and the v2-shaped unpacked gate, which must be NOT RESOLVED** (G10d) |
| `log_enabled_0args` / `_2u32` / `_str32` | ≤ 15 / 20 / 30 ns | runtime-disabled |
| `log_enabled_rate_once_fired` | ≤ 5 ns, **no store, no shared line** | `Every` policy |
| `sink_sustained_rate` | finds the drop knee; reports records·s⁻¹ | zero-record idle sink |
| `lane_padding_ablation` | padded+cached vs padded-only vs neither | — |
| `sched_cpu_logger_on_off` (gate **P1**, re-specified) | not resolvable above the floor | interleaved zero control, ABBA |
| `log_dyn_disabled` | ≤ 4 ns, **and the delta vs `log_disabled_runtime` must RESOLVE** | `log_disabled_runtime`, same sitting |
| `log_enabled_sampled_out` | ≤ 6 ns | the same site with shift = 0 |
| `log_enabled_0args_sampling` | NOT RESOLVED vs the pre-L12 baseline | pre-L12 baseline, same sitting |
| `log_pod_12b` | ≤ 20 ns | `dsp!` of the same value, which must be ≥ 5× slower |
| `sink_sustained_rate_binary` | ≥ 3 M rec·s⁻¹ **and ≥ 5× the text sink** | the text sink, same sitting |
| `downstream_code_warn` | ≤ 18 ns | the engine-code `warn!`, same sitting; the delta is the `idx_cell` load |

**`log_disabled_compile` is deleted** (B7). A compile-disabled site optimises to nothing and the loop body is empty, so the bench measured the control against the control; it could not go red, and adding `black_box` around the arguments would falsify the very "0 argument evaluations" property under test. The property is proved by **G1** (symbol gate) and **G4** (side-effect probe), both of which have named red states.

### P1 is re-specified: the old instrument could not respond to the quantity *(fixes F3)*

v2's P1 measured **windowed frame time** with the logger on vs off. `VK_PRESENT_MODE_FIFO_KHR` is unconditional (`crates/boyko_rhi_vulkan/src/present/swapchain.rs:199`, with only a support check at `:164`), so wall-clock frame time is clamped at the refresh interval; the wall floor here is 6.893 ms. A logger idling — or even emitting 1000 records/frame at 15 ns — perturbs CPU work by ≲ 15 µs, ~0.2 % of a clamped frame. **The channel is structurally incapable of responding**, so P1's outcome was pre-determined `NOT RESOLVED`. That is the P4-6 lesson repeated: an instrument whose band is structurally zero reports `RESOLVED`/`NOT RESOLVED` without measuring anything.

Worse, open question 4 then proposed to read that silence as a positive result — "if P1 comes back `NOT RESOLVED` the thread is free" — the exact inference this document forbids three pages earlier ("absence is `UNPROVEN`, never `clean`"). Both are fixed:

- **P1 becomes a headless CPU-work bench.** `crates/bench_bevy_vs_boyko` already runs schedules with no swapchain and no FIFO clamp. P1 runs the same schedule logger-on vs logger-off, ABBA-counterbalanced, with an interleaved zero control (A0 vs A1) in the same sitting, and reports `RESOLVED(x %)` or `NOT RESOLVED` against the sitting's own floor. **What P1 cannot claim**: anything about a *windowed* frame, where present pacing dominates. That question is P2's, and P2 can only claim drift and leak, not perturbation.
- **The inference is struck.** `NOT RESOLVED` from P1 means **UNPROVEN**, and open question 4 no longer offers it as a licence. The sink thread's disposition in a shipping title is decided **structurally** — `shipping-min` exists precisely so the owner has an off switch that does not depend on a measurement this hardware may not be able to make.

### Gates — every one has a RED that can be SHOWN, and a stated limit

v2's G1-G5 carry forward unchanged in substance (G1 relocated to L1-gate). G2, G7 and P1 are re-specified above and in Decision 3. New and re-cut gates:

| # | Gate | RED variant that must be demonstrated once | **What this gate CANNOT claim** |
|---|---|---|---|
| **G17** | **Error-reserve arithmetic, three-sided** *(the F6 regression gate)*. Fill a lane past `LANE_BYTES − ERROR_RESERVE` with `Trace`, then: (a) a further `Trace` is **refused and counted**, not written; (b) an `Error` still lands; (c) the neighbouring lane's poison canary bytes are untouched | **replace `avail.saturating_sub(ERROR_RESERVE)` with `limit - used`** ⇒ (a) fails (the `Trace` is written) and (c) fails (the canary is clobbered). This is v2's exact code, so the gate reds on the shipped defect | It cannot claim the ring is correct under *concurrent* drain — that is test 7's job (live producer). G17 drives a quiesced consumer on purpose, so the arithmetic is the only variable |
| **G18** | **`OUT_LOCK`, two-sided** *(the F8 gate)*. (a) A thread that acquires the lock and then panics releases it — a second thread's `report!` completes within the deadline; (b) a re-entrant `report!` from inside a sink panic handler **completes** and increments `OUT_REENTRANT` | replace the RAII guard with a bare `store(false)` after the write ⇒ (a) hangs and the test's own deadline reds it | It cannot claim output is never interleaved. Under a **steal** it is, deliberately. `OUT_STEALS > 0` in the census is the honest report; a nonzero value in a golden run is itself a defect signal |
| **G8** | **Dynamic targets, four-sided.** (a) A registered dynamic target's records arrive and appear in the census under its interned name; (b) registration past the 32-slot band returns `None` + `E0106`; (c) re-registering a name returns the same id; (d) the **bench** leg: `log_dyn_disabled` − `log_disabled_runtime` must **RESOLVE** above the sitting's floor | make `register_dynamic_target` grow past the band ⇒ (b) fails; share one slot for two names ⇒ (c) fails | It cannot claim the dynamic path is cheap enough for a hot loop — it bounds it at ≤ 4 ns disabled. **If (d) does not resolve, Decision 2's claim that gate (a) buys anything is STRUCK from this document** rather than restated |
| **G9** | **Downstream code minting.** 16 threads mint one downstream code concurrently ⇒ exactly one dense index, no leaked counter value, `CODE_OCCUPANCY` advanced by exactly 1 | swap the reserve/`fetch_add` order ⇒ indices leak; the density assertion fails | It proves nothing about a game's *registry completeness* — the eight checks are engine-scope and `codes_tidy!` must be invoked by the game. Written into the gate's own assertion message |
| **G9b** | **`LogPod` round-trip.** A `#[derive(LogPod)]` struct with padding encodes/decodes byte-identically; and a hand-written impl whose `POD_LEN` lies is caught by a `debug_assert` + Miri | drop the `POD_LEN == size_of::<Self>()` assert ⇒ the padded-struct Miri leg reports uninitialised reads | It cannot make `unsafe impl LogPod` safe for an arbitrary hand impl. The derive is the documented route |
| **G10** | **Sampling: exactness + non-perturbation.** (a) shift = k over N records on one lane ⇒ delivered == `N >> k` **exactly** and `sampled_out == N − (N >> k)` exactly; (b) control leg shift = 0 delivers all N; (c) 8 threads × 8 targets: every (lane,target) pair independent; (d) **perturbation**: `log_enabled_0args` NOT RESOLVED vs the pre-L12 baseline | delete the `& mask` ⇒ (a) fails; share one counter across lanes ⇒ (c) fails | It cannot claim a sampled capture is *representative*: `1/2^k` is strided, not random. **If (d) resolves, `log-sampling` becomes default-off and the ≤ 15 ns row is annotated with the measured cost** — the gate decides the rung, the plan does not pre-decide it |
| **G11** | **Drop honesty at scale, two-sided.** With `preset_drop_counter(u32::MAX − 3)`: offering 100 records leaves the counter at `u32::MAX` (never wrapped) and the census prints `SATURATED`; with the counter at 0, offering 100 dropped records prints exactly `100` | remove the saturation guard ⇒ the counter wraps to a small number and the "never less than before" assertion fails | It cannot claim `LogStats.dropped` (u64) is exact *between* the last drain and process death — in that window the per-lane `u32` is the only record. The census says `since_last_drain` explicitly |
| **G12** | **Binary sink, three-sided.** (a) round-trip byte-identical to the text sink for the same records; (b) a record straddling a rotation appears exactly once and every rotated file decodes standalone; (c) throughput ≥ 5× the text sink in the same sitting | omit a dictionary record ⇒ (a)'s decoder cannot resolve a site; skip the dictionary re-emit on rotation ⇒ (b) fails on file `.1` | It cannot claim cross-version compatibility — the decoder **refuses** a `schema_version` mismatch. **If (c) does not separate, L13b is REVERTED** |
| **G13** | **Runtime sink control, three-sided.** (a) enabling a file sink mid-run from a non-sink thread: records before are absent, records after are present; (b) the requesting thread's **per-thread** allocation counter reads zero and no `open` occurs on it; (c) a deliberately blocking `CallbackSink` causes lane drops that are **counted** | perform the `open` on the requesting thread ⇒ (b) fails; silence the callback-stall drops ⇒ (c) fails | It cannot claim a filter change is instantaneous: a sink acts on the filter it read at the top of its current drain, so the boundary is fuzzy by up to one drain. Pinned as a property |
| **G14** | **Crash drain, two-sided.** (a) `SINK_STATE = Exited`, panic after an `error!` ⇒ the record reaches the crash file + `E0109`; (b) `SINK_STATE = Running` ⇒ the crash path does **not** take the role (`E0109` absent) and a per-record uniqueness check over both files shows **no record delivered twice** | remove the `SINK_STATE` CAS ⇒ (b)'s uniqueness check fails (two consumers) | It cannot claim survival of `abort()`, `SIGSEGV` or a guard-page stack overflow — the hook does not run (E22). Partial mitigations named with their limits |
| **G15** | **In-frame consumption, two-sided.** (a) a record emitted in frame N is visible to a system reading `LogRing::since` by frame N+2; (b) it is **not** visible before the drain that consumed it | feed `LogRing` from the emit path ⇒ (b) fails (and would have coupled the hot path to ECS storage) | It cannot claim a bound tighter than "sink park interval + one frame". `LogRingIter::skipped` reports ring-wrap loss so a console cannot silently miss lines |
| **G16** | **Shipping profile symbol gate, two-sided.** In the `shipping` CI leg no `emit_impl` monomorphisation reachable from a `debug!`/`trace!` fixture appears; in `dev` it **must** appear | drop the `GLOBAL_CEILING` gate ⇒ the symbol appears in `shipping` | It cannot claim a *dynamic* site is compiled out per-target — dynamic sites have no gate (a); they are deleted only by `GLOBAL_CEILING`, which is why the fixture includes a `dyn_debug!` site |
| **P2** | **30-minute soak, `shipping` profile, 5 K rec·s⁻¹.** (a) `dropped == 0`; (b) resident bytes flat between minute 5 and minute 30; (c) windowed frame time vs logger-off, ABBA + interleaved zero control | leak one buffer per rotation ⇒ (b) fails; shrink the ring ⇒ (a) fails, which is the intended positive control for the drop counter at session scale | (c) **cannot resolve a CPU perturbation** — FIFO clamps it (F3). P2's (c) leg is retained only as a **drift/leak** check and is labelled as such in the artifact; the perturbation question belongs to P1's headless channel. P2 also cannot claim anything about a different game's emission profile — the load is synthetic and the artifact records its shape |

**Where no control is possible, it is written down rather than worked around**: `boyko-W0101` (invariant TSC absent) has no reachable red state on any targeted machine (N30, `tests/untested_codes.txt`); a forced `SYNC-HAZARD` is unavailable on this box (M25; G7 uses an ordinary validation error instead); a *chained* validation-features node is unbuildable here (F2; G7's negative leg is validation-off instead); and a hard-crash (non-unwinding) log tail is unobtainable by construction (G14's stated limit).

### Mandatory tests
1. **G4 — four-way gate separability** (§L0-gate). Each gate has its own red state; the enabled leg must reach **1000**, not merely `0` when disabled; the shift = 1 leg must reach exactly **500**.
2. **G1 — symbol gate** (at **L1**-gate, not L0 — F19). Disabled fixture: no `emit_impl` symbol. Armed fixture: symbol present. Red state: delete a gate.
3. **Allocations on the producer path — steady state 0, first emit ≤ 1, and the difference is stated** *(fixes F26)*. Via a **per-thread** counting allocator (`thread_local! { static N: Cell<u64> = const { Cell::new(0) } }` — const-init, no `Drop`, no TLS registration, no allocation of its own). Three legs:
   - **(a) steady state**: arm *after* a warm-up emit on the same thread; assert exactly **0**. Red state: make `encode` allocate.
   - **(b) first emit**: arm on a **fresh** thread *before* its first emit; assert **≤ 1** and record the observed number. The lane guard is a `thread_local!` with a `Drop`, so destructor registration is on the first-emit path on some platforms — v2's blanket "0 allocations on the producer path" was true only of the steady state and did not say so.
   - **(c) monotonicity**: 1000 emits on that fresh thread must not raise the count above leg (b)'s value. A per-emit allocation would show here even if leg (a)'s warm-up hid it.
   This covers `SinkMode::Thread`, which the process-global counter structurally cannot: `crates/boyko_ui/tests/zero_alloc.rs:44-60` had to add `ARM_LOCK` after observing an impossible **negative** delta from a sibling thread, and a permanently resident sink thread cannot be serialised by a test-local lock (M18). The process-global variant is retained as a second, `Manual`-mode gate with its limitation stated.
4. **Overflow drops and counts** — fill a lane, assert `dropped > 0`, exactly one `W0102` per drain with matching counts.
5. **Error reserve** — flood with `Trace`, assert a subsequent `Error` still lands.
6. **Wrap protocol** — proptest over record sizes crossing every tail offset in `LANE_BYTES-32 ..= LANE_BYTES`; assert no byte is written outside the lane (poison the neighbouring lane's guard bytes and check them), and that producer and consumer agree on every PAD.
7. **Staged drain under a LIVE producer** (B1's red state) — a producer running at full rate while the sink drains; assert every decoded record is byte-identical to what was offered. **v1's design fails this test; v1's tests never ran it, because both drove a quiesced producer.**
8. **Lane claim/retire** — 200 short-lived threads against 128 lanes; assert every lane eventually returns to `FREE`, **and** assert `Warn`/`Error` from unlaned threads reached the synchronous fallback (M26). Reclaim is asynchronous, so the assertion is "eventually, within a bounded flush", not "immediately".
9. **Flush without a consumer** returns `NoConsumer` immediately; **flush timeout** returns within 2 s with `E0105`; **shutdown** detaches on timeout with `E0108`.
10. **Panic hook flushes** — `catch_unwind` around a panic after an `error!`.
11. **Registry: the eight checks**, each shown red once during development, over the three-stream walker.
12. **Rate policies** — `Once` (incl. the no-store property), `EveryN`, `MinInterval`, `suppressed_since_last`.
13. **Census** — `UNPROVEN` at 0 records **and** `UNPROVEN(lossy)` at `dropped > 0`.
14. **Miri (Tree Borrows)** — ring, claim CAS, typed header round-trip incl. `*const LogSite` provenance, staged copy.
15. **loom** — claim/retire and the cursor pair. (Loom *release* binaries crash at startup on this box, pre-existing; run loom in debug.)
16. **Machine-API preservation, concurrent** (M24, corrected by F13) — `report!` output byte-identical to today's `VB-P1d`/`VB-P4` lines **while the sink floods stderr and a `println!` writes stdout concurrently**. The merged-stream consumer is `scripts/golden.ps1:196-202` (a `cmd /c … > log 2>&1` wrapper, not a plain `2>&1`); `vg_occ_split_timing.rs:1115-1117` reads `out.stdout` and `out.stderr` as **separate buffers** and is therefore structurally immune to cross-stream interleaving — v2 attributed the justification to the wrong consumer. The property actually under test is **intra-stdout** ordering between `report!` and surviving `println!` sites, which is why `report!` writes through `stdout()`'s own `LineWriter` (F17). Red state: give `report!` a raw handle ⇒ a `println!` line splits around a `report!` line.
17. **`[vk-validation]` liveness and byte-exactness — with a POSITIVE control, because zero messages is this machine's measured normal** *(fixes F1)*. v2's test 17 said "`golden.ps1`'s grep matches, and the message is on the wire before the frame returns" — but `golden.ps1:226` matches nothing and `:232` prints "clean (0 messages)" in green at **zero**, which is the steady state here (a genuine missed barrier produced zero messages, twice). It could not distinguish "the prefix survived" from "no message existed", and its second clause named no observation mechanism at all. v3:
    - **(a) positive control**: a fixture makes a deliberately invalid call that produces ≥ 1 ordinary validation message. Assert `count ≥ 1` and that the line begins with the byte-exact `"[vk-validation] "` **including the trailing space**, pinned against `crates/boyko_app/tests/vb_bench_query_validation.rs:116-118`'s constant.
    - **(b) ordering, with a mechanism**: immediately after the offending call returns, the fixture writes a synchronous marker line. Assert the `[vk-validation]` line **precedes** the marker in the merged stream. Red state: buffer the messenger ⇒ the marker comes first.
    - **(c) negative**: a run with no invalid call produces zero `[vk-validation]` lines and the census reports `status=UNPROVEN`, **not** clean.
    - **What test 17 cannot claim**: that a run with zero messages is a run with no defects. It never could; §sync-validation confrontation is the standing statement of that.
18. **Dynamic target interning** — 32 registrations succeed with distinct ids; the 33rd returns `None` with `E0106`; re-registering an existing name returns the same id; concurrent registration of one name from 16 threads yields one id.
19. **Cursor wrap at 2³²** (E17) — preset `write`/`read` to `u32::MAX − 64`, push records across the boundary, assert every record decodes and `write.wrapping_sub(read) <= LANE_BYTES` throughout.
20. **`LogRing` cursor wrap** — same treatment for `head` / `arena_cursor` / `seq`, with `skipped` reported.
21. **Saturating drop counters** (E18) — see G11.
22. **Sampling exactness and independence** — see G10 (a)(b)(c).
23. **Downstream code minting under contention** — see G9.
24. **`LogPod` under Miri (Tree Borrows)** — a padded `#[repr(C)]` struct round-trips; a `fmt_pod` reading beyond `POD_LEN` is caught; **and an assertion that no user code executes between lane acquisition and the `Release` store** (a `LogPod` whose `fmt_pod` sets a TLS flag; the flag must be unset at the `Release` store and set only during drain).
25. **Sink filter and `UNPROVEN(unsunk)`** — a target enabled at `Info` with no sink accepting it produces `status=UNPROVEN(unsunk)` + `W0111`, not silence.
26. **Runtime sink open/close** — see G13.
27. **Crash drain, both sides** — see G14.
28. **Binary round-trip and rotation** — see G12 (a)(b).
29. **`EPOCH` correlation** — every record's `tsc` falls between the epochs of exactly one frame; a record emitted during the drain itself is attributed to the next frame, and that is **asserted, not assumed**.
30. **Control spec parsing** — `apply_control_spec("net=debug/6!, ecs=off")` sets level, shift and sync bit for the named targets, bumps `CONTROL_EPOCH` by exactly 1, leaves unnamed targets **bit-identical**, and rejects an unknown name with a coded error rather than silently ignoring it.
31. **Per-site `Once`** (F11) — three sites sharing one code, all three fire exactly once; the same site called 10⁶ times fires once and performs **no store** after the first.

### Property-based
- Random `(level, target, arg-tuple)` sequences round-trip byte-identically through `encode`/`decode`, **including `LogPod` members**.
- Random fill/drain interleavings: `emitted == drained + dropped + sampled_out`, exactly, always. *(`sampled_out` is a separate term precisely so this identity stays exact — folding it into `dropped` would have made the drop count a liar in the other direction.)*
- For any rotation schedule and any record stream, `logdec` over all retained files yields a subsequence of the emitted stream with no duplicates and no reordering within a file.
- For any control-spec string, `apply_control_spec` is idempotent: applying it twice yields bit-identical `CONTROL`.
- **For any `(used, need, level)` with `used <= CAPACITY`, the admission arithmetic never admits a write that would pass `read`** — the property F6 violated, stated as a proptest over the raw integers rather than only as a scenario test.

### `debug_assert!` invariants
`len <= MAX_RECORD_BYTES`; `len == HEADER_BYTES + args.encoded_len()`; `write.wrapping_sub(read) <= CAPACITY`; `MY_LANE < MAX_LANES || == NONE`; `!IN_EMIT.replace(true)` (re-entrancy); `drain()` only under `Manual` **or while holding the consumer role**; `code_idx < MAX_CODES`; `boot()` at most once; `codes!` strictly increasing (also a compile-time `const _`); `EveryN(n).is_power_of_two()` (compile-time); `sample_shift <= 15`; `LogPod::POD_LEN == size_of::<Self>()`; `SAMPLE_CTR` row index == `MY_LANE`; `SINK_STATE` transition is a permitted edge. *(`TargetId < MAX_TARGETS` is **not** in this list any more: it is now a type invariant upheld by a closed constructor set, so there is nothing to assert at use — F15.)*

**Release-live** (these run in every profile, not only debug): the `MAX_RECORD_BYTES` check, the admission-control `saturating_sub`, the drop-counter saturation guard, the sampling arithmetic, the census status computation, the `SINK_STATE` CAS in the crash path, `OUT_LOCK`'s acquire deadline, and the `SINK_REQ`-full refusal.

---

## Edge cases

| # | Case | Behaviour |
|---|---|---|
| E1 | Log before `boot()` | `CONTROL` is `.bss`-zero = level `Off`, shift 0, sync 0; one L1 load, one `and`, not-taken branch. Correct and free. |
| E2 | Log after shutdown | Lanes accept; nothing drains; `dropped` climbs **to saturation, then stops** (E18); census reports lossy. Shutdown flushes first. |
| E3 | Record over `MAX_RECORD_BYTES` | **Runtime** check, every profile: dropped, `TOO_LARGE` flag, counted. Not a debug panic reachable from safe code (N29). |
| E4 | Ring exactly full | One-slot-reserved convention distinguishes full from empty without a third variable. |
| E5 | Tail too short for a record | PAD record if `tail >= HEADER_BYTES`, else the shared implicit-wrap rule. Both sides apply the same rule (B3). |
| E6 | 129th concurrent logging thread (33rd in `shipping`) | `Warn`/`Error` go to the synchronous fallback; lower levels count into `UNLANED_DROPPED`; census reports it (M26). |
| E7 | Thread dies mid-record | Impossible to publish a partial record: header+payload write and the `Release` store are straight-line with no yield point. A thread killed between them leaves `write` unmoved; the bytes are overwritten. |
| E8 | `tsc` wrap | 64-bit invariant TSC at ~3 GHz wraps in ~195 years. Not handled; stated. |
| E9 | Non-monotonic clock across sockets | Merge order degrades to approximate — already the documented property. Single-socket assumption stated. |
| E10 | `&str` > 256 B | Truncated, `STR_TRUNCATED`, sink appends `…[truncated]`. |
| E11 | Two engine targets claim one ID | **Does not compile** (Decision 15). Downstream collision ⇒ `boyko-E0104` at boot, naming both. |
| E12 | File sink hits `max_bytes` | `Rotation::NONE` (engine default): one `boyko-W0103`, writing stops, other sinks continue. `Rotation{keep}`: rotate, delete the oldest, re-emit anchor + dictionary, `boyko-W0112` **naming the deleted file and the record range lost**. |
| E13 | Validation storm | Unchanged from today: `eprintln!` on `stderr()`'s own lock, synchronous, never dropped (Decision 9b — the site is not edited). |
| E14 | Panic inside a sink | The sink catches, direct-writes, continues — and the direct-write **cannot self-deadlock**, because `OUT_LOCK` detects same-thread re-entrancy and writes with a leading newline instead of spinning (Decision 9c, G18b). A sink must not kill the process. A sink that faults repeatedly is set `Faulted` and skipped; other sinks continue. |
| E15 | `flush()` from two threads | Distinct epochs via `AcqRel fetch_add`; both return. |
| E16 | Drop order at shutdown | `shutdown()` → `flush()` → `Exiting` → unpark → bounded spin on `SINK_EXITED` → detach on timeout → sinks close. Idempotent; safe from `App` teardown, the panic hook and process exit. |
| **E17** | **Lane cursor wraps `u32`** (~2.4 h at 500 KB·s⁻¹·lane) | **Correct, and proved**: every comparison is `wrapping_sub`, every index is `& MASK`, and `w − r ≤ CAPACITY ≪ 2³¹` so the unsigned difference is unambiguous across one wrap. Test 19 presets the cursors at the boundary. |
| **E18** | **Drop counter reaches `u32::MAX`** (only possible with no drain) | Saturates; census prints `dropped=SATURATED(>=4294967295)` — never a number a reader could compare. `LogStats.dropped: u64` carries the across-drain total (Decision 21, G11). |
| **E19** | **33rd dynamic target** | `register_dynamic_target` returns `None` and `boyko-E0106` names the rejected string. There is no `TargetId` to misuse afterwards, because absence is `Option<TargetId>` (F15) — the "emitted on an invalid id" case is unrepresentable rather than counted. |
| **E20** | **A target is enabled but no sink accepts it** | Census `status=UNPROVEN(unsunk)` + `boyko-W0111` once. Without this, a game enables a category, sees an empty log, and concludes "clean" — the vacuous gate in a new costume. |
| **E21** | **Rotation deletes evidence** | `boyko-W0112` names the deleted file and the record range lost. Every retained file is independently decodable (anchor + dictionary re-emitted). A capture that silently discards its own beginning is not a capture. |
| **E22** | **Hard crash — `abort()`, `SIGSEGV`, guard-page stack overflow** | The panic hook does not run; whatever is in the lanes and in `SINK_OUT` is lost. **No design in this crate survives it**; making every record durable is a syscall per record. Bounded, not fixed, by: the per-target sync bit (~200 ns, durable-on-write), `flush_interval_ms` (default 1000 in `shipping`), and the boot-opened crash file, which at least exists and carries the session header. Stated, not worked around. |
| **E23** | **Sampled capture aliases a periodic emitter** | `1/2^k` is strided, not random. Per-lane seeding breaks aliasing across lanes, not within one. The census prints `sampling=1/N (strided, not random)` and `boyko-W0113` fires once per sampled target, so the bias is in the log rather than only in a footnote. |
| **E24** | **A `CallbackSink` blocks** | The sink thread stalls; lanes fill; drops are counted and reported. This is the stated cost of putting a network/telemetry sink behind the callback seam rather than inside `boyko_log` (§Refused), and G13c demonstrates it rather than leaving it as prose. |
| **E25** | **`OUT_LOCK` acquire times out** | The writer **steals**: it writes anyway, increments `OUT_STEALS`, and emits `boyko-W0110` once. An interleaved line is a legible defect; a hung process is not (Decision 9c). |

---

## Open questions

1. **`report!` schema.** The tree has a flat-TOML + `schema_version` artifact convention (`vb_probe_dump.rs:183-194`) and **no timing output uses it**. Should `report!` gain a second schema-versioned TOML form alongside the byte-frozen text? SCOPE call; this plan preserves the text verbatim and adds nothing.
2. **`--explain` delivery.** This plan embeds a one-line `summary` and requires `docs/diagnostics/<code>.md` (check 2). It does not add a `boyko-explain` binary. rustc's registry stays honest partly *because* three consumers read one table; we have two. Does the owner want the third?
3. **`.bss` budget.** Decision 3's matrix: ≈ 3.4 MiB reserved for `dev`, ≈ 1.1 MiB for `shipping`, demand-zero, resident a small fraction. Acceptable, or should the `dev` profile default to 64 × 8 KiB lanes?
4. **Sink thread in shipping builds.** One extra OS thread, idle-parking at 125 Hz. **v2's inference here is struck** *(F3)*: it said "if P1 comes back `NOT RESOLVED` the thread is free", which is the reading of silence as proof that this document forbids everywhere else. `NOT RESOLVED` means **UNPROVEN**. The question to the owner is therefore a VALUES call and not a measurement: is one parked OS thread acceptable in a shipped title, given that `shipping-min` exists as a `SinkMode::Manual` profile with no resident thread at all?
5. **The `15xx` / `90xx` block split** is grandfathered and tidying is forbidden. Confirm — renumbering breaks the book, the `#[should_panic]` assertions and the never-reuse rule at once.
6. **The one-line RHI fix for sync-validation** (`pLayerName = "VK_LAYER_KHRONOS_validation"`) is deliberately not in this plan. It has a large blast radius: sync-validation coming alive would surface real hazards and could turn every golden run red. Pull it into L7, or keep it a separate RHI item coded by `E2101`? **Note that G7's negative leg is re-cut around its absence** (F2), so answering this does not block L7.
7. **`OUT_LOCK`'s registration is now moot** *(resolved by F9)*. v2 asked the owner to confirm a `docs/HOT-PATH-EXCEPTIONS.md` entry. That entry is **not implementable**: `scripts/check_hotpath_exceptions.py` matches rows against `#[allow(clippy::disallowed_types)]` counts per file, and an `AtomicU64` carries none, so the row would red CI. The question that remains is narrower: **confirm the steal-on-timeout trade** (Decision 9c) — a possibly-interleaved line instead of a possible hang — rather than an attempt to make the channel lock-free, which is strictly more machinery for a path that is cold by construction.
8. **Dynamic band size.** 32 slots (IDs 224..=255). A modding-heavy title could want more, but every extra slot comes out of the 256-target space that `CONTROL`, the sink filters (`[u64; 4]`) and `TARGET_STATS` are all sized by; past 256 those three arrays become two-level structures. **Recommendation: ship 32 and treat "more than 32 data-defined categories" as a signal that the taxonomy belongs in source.** VALUES call.
9. **Is `log-sampling` default-on?** Decided by G10d, not by this document. Flagged so the answer is recorded rather than absorbed.
10. **Does the engine profile want rotation?** `Rotation::NONE` is kept as the engine default so a bench cannot silently discard its own beginning. A long editor session would want rotation. **Recommendation: `dev` keeps `NONE`; a future `editor` profile gets rotation.** SCOPE call.
11. **Telemetry payload shape.** `LogCensus` gives a game per-target counts; the binary log gives it everything. What it does *not* give is a compact per-session summary suitable for upload (a few hundred bytes). That is a game-side reduction over `LogCensus`, deliberately not designed here — but if the owner wants a canonical shape shipped, it is a small additive rung after L16.
12. **Should `boyko_ui`'s console (L9) live in this plan or the UI plan?** L16 now fixes the entire contract it consumes (`since`, `RingFilter`, `skipped`, `LogCensus`, `CONTROL_EPOCH`), so L9 is a pure UI rung with no logging design left in it. Recommend moving it wholly to the UI plan and deleting L9 from this ladder.

---

## Checklist

**Structure** — goal in perf+functional terms, both audiences ✔ · concrete targets with named red-state controls ✔ · every decision justified via perf/cache/parallelism ✔ · alternatives rejected with reasons ✔ · trade-offs listed ✔ · the five audience conflicts named, decided, and costed for the losing side ✔ · eight asks explicitly refused with reasons ✔
**Data structures** — every field typed + commented ✔ · `repr`/`align`/`packed`/`transparent` where it matters ✔ · three-partition cache-line split ✔ · sizes pinned by `const _: () = assert!`, **including the `COMMIT_GRANULE` divisibility pin that v2 violated** ✔ · false-sharing padding specified for `Lane`, `RateSlot`, `DynSlot`, `TargetStatCell`, `SinkSlot` ✔ · producer working set still ≤ 4 lines after the extension ✔
**API** — minimal ✔ · no internal types in signatures ✔ · lifetimes trivial ✔ · no `dyn` anywhere (the callback seam is an `extern "C" fn` + ctx) ✔ · generics where specialisation is needed (`emit_impl<A: LogArgs>`, the `LogPod` blanket) ✔ · **no public type can hold an out-of-range index** ✔
**Multithreading** — model explicit ✔ · every atomic's ordering stated, including the new data ✔ · **every wait bounded, including `OUT_LOCK`'s** ✔ · the one sync point short-circuited when no consumer exists ✔ · the crash path's consumer-role transfer proved by a CAS out of three provably-quiescent states, not by a timeout ✔ · `Send`/`Sync` consistent ✔ · race-freedom argued, including the single-thread re-entrant case and `LogPod` ✔
**Correctness** — 25 edge cases ✔ · **the admission arithmetic proved by induction and proptested over raw integers** ✔ · session-scale integer audit with a per-quantity table ✔ · lane-`owner` protocol replaces generation checks ✔ · drop order (E16) ✔ · `unsafe` invariants in the `Sync` SAFETY block (clauses 1-4 incl. 1c-1f) and per algorithm ✔
**Integration** — machine-generated ledger, not a hand table ✔ · census arithmetic reconciled and its denominator named ✔ · API changes explicit ✔ · `Arena`/`ComponentPool`/`UnitId` untouched ✔ · no stdout contract moves ✔ · 21 rungs, each landing green and committing alone, each with a "must NOT move" column ✔
**Validation** — 31 tests ✔ · 5 property families ✔ · 12 benches with in-sitting controls ✔ · `debug_assert!` **and release-live** invariant lists ✔ · every gate has a named red state **and an explicit "what this gate CANNOT claim"** ✔ · three gates (G8d, G10d, G12c) can **revert their own rung** ✔ · **one gate (G17) exists to red on a defect this document previously shipped** ✔

---

## Findings disposition (v1 → v2 review) — carried forward unchanged

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

**Refuted: none** (of B1-N32). Every one was either a defect in v1's own pseudocode, a gate that could not fail, or a claim v1 made without a control. Two of these folds were themselves defective and are re-folded below: **M13** (F7 — the fold did not run), **M25** (F2 — the fold installed a second unshowable control). **M11** and **M24** are amended (F10, F13).

---

## Findings disposition (v2 → v3 review, verdict REJECTED)

| # | Finding | Disposition | Where in v3 |
|---|---|---|---|
| **F1** | Test 17 has no red state; its green is this machine's measured normal | **FOLDED** | §Metrics test 17 — positive control (deliberate invalid call, `count ≥ 1`), byte-exact prefix pinned against `vb_bench_query_validation.rs:116-118`, ordering proved by a synchronous marker line, plus an explicit "cannot claim" |
| **F2** | G7's negative side is unshowable here; M25 reintroduced by its own fold | **FOLDED** | §sync-validation confrontation + L7-gate — the two sides become validation-**on** ⇒ fires / validation-**off** (`BOYKO_DISABLE_VALIDATION=1`) ⇒ absent. Both runnable today. "Cannot claim" names the chained case as out of reach |
| **F3** | P1 cannot resolve (FIFO-clamped), and open question 4 reads its silence as proof | **FOLDED** | §Metrics "P1 is re-specified" — headless schedule bench (`bench_bevy_vs_boyko`), not windowed frame time; FIFO confirmed unconditional at `present/swapchain.rs:199`. The inference is **struck** from open question 4; P2's frame-time leg is relabelled drift/leak-only |
| **F4** | G2's two non-size legs have no mechanism; the size leg is a const tautology | **FOLDED** | §Decision 3 — a three-row mechanism table: OS thread count (toolhelp / `/proc/self/task`) **with its own rising-count control**, behavioural panic-hook probe, and the size leg kept but annotated as env-plumbing proof only |
| **F5** | Registry check 3 satisfiable by a doc comment; checks 3 and 6 need opposite walkers | **FOLDED** | §Decision 6 — ONE walker, THREE streams (CODE / LIT / TEXT); each check names the streams it consumes. Measured corpus: 18 doc-comment `B1802` sites in `app.rs` (**not 28** — count corrected, finding stands), plus 6 in `schedule_builder.rs` |
| **F6** | `free` UNDERFLOWS in Algorithms A5; the producer overruns live ring bytes | **FOLDED (correctness bug)** | §Decision 5 + §Algorithms A6 — `avail = CAPACITY − used` (inductive, cannot underflow) and `budget = avail.saturating_sub(ERROR_RESERVE)`. **Gate G17 reds on v2's exact code**; a proptest covers the raw integers |
| **F7** | `LogRing`'s `VmColumn<LogLine>` assert-panics at construction; M13's fold does not run | **FOLDED (correctness bug)** | §Data structures — `LogLine` is **16 B**, `#[derive(Clone, Copy)]`, with `const _: () = assert!(COMMIT_GRANULE % size_of::<LogLine>() == 0)`. Verified against `vm_column.rs:144-149` and `constants.rs:7`. `TargetStat` gets the same pin (32 B). `pub(crate)` visibility is a non-issue: `LogRing` lives inside `boyko_ecs` and the field is private |
| **F8** | `OUT_LOCK` is an unbounded spin with no poison story; Invariant 6 violated | **FOLDED** | §Decision 9c — format-before-lock, re-entrancy detection via `OUT_OWNER`, 50 ms bounded acquire then **steal**, RAII release on unwind. Gate G18 two-sided. E25 states the steal |
| **F9** | `OUT_LOCK` cannot be registered in `HOT-PATH-EXCEPTIONS.md`; doing so reds CI | **FOLDED** | §Invariant 1 — verified against `scripts/check_hotpath_exceptions.py:15-19,51,337-341`. **No row is added.** Open question 7 is rewritten from "confirm the entry" to "confirm the steal trade" |
| **F10** | `Once` suppression is invisible to the census too; contradicts the Goal | **FOLDED** | §Goal bullet 3 + §Decision 8 — three quantities kept separate; the census prints `suppressed=UNCOUNTED(by policy)` so the *absence of a count is itself printed*; `OnceCounted` offered with its RMW cost stated at the declaration site |
| **F11** | Rate policy is code-scoped while one code covers N sites; `W2102` silences two of three | **FOLDED** | §Decision 8 — `Once`/`OnceCounted` become **per-SITE** (a macro-generated `static FIRED` beside the `LogSite`). Cheaper too: a private line instead of a shared `RATE` line. Test 31; behaviour-ledger rows for `W2102`/`W2202` updated |
| **F12** | Decision 9b's justification for touching `debug.rs` is fictitious and its edit changes bytes | **FOLDED** | §Decision 9b — the edit is **withdrawn entirely**; the site is untouched and allowlisted with a reason. `Cow::Borrowed` (no allocation), byte change only in the invalid-UTF-8 case, and the `stderr()`-lock regression are all named |
| **F13** | M24's "verified" claim misdescribes its own consumer | **FOLDED** | §Invariant 2 + §Compatibility + test 16 — `vg_occ_split_timing.rs:1115-1117` reads `stdout`/`stderr` as separate buffers; the merged consumer is `golden.ps1:196-202` via `cmd /c`. The real hazard is intra-stdout (F17) |
| **F14** | Machine-consumer inventory incomplete; a third gate's entire input uncited | **FOLDED** | §Invariant 2 — measured inventory: **31 files** reference `[vk-validation]`, **16** reference `VB-P1d`. `vb_bench_query_validation.rs:116-118`'s byte-exact constant (trailing space) is now pinned by test 17 |
| **F15** | `TargetId(pub u16)` vs `MAX_TARGETS` with a debug-only bound | **FOLDED** | §Decision 15 + §Data structures — private field, closed constructor set, `get_unchecked` sound with a SAFETY clause naming it. `INVALID` deleted in favour of `Option<TargetId>`; E19 rewritten; the `debug_assert` removed as vacuous |
| **F16** | Principle 0 exception neither named nor argued | **FOLDED** | §Invariant 7 — claimed in writing on dependency inversion, with the cost (no `Query`, no change detection) stated and `CONTROL_EPOCH` named as the mitigation |
| **F17** | Buffered-stdout race between `report!` and the surviving `println!` sites | **FOLDED** | §Decision 9 — `report!` writes **through `stdout()`'s own `LineWriter`**, not a raw handle; cost (one memcpy + one flush) stated. Test 16's red state is the raw-handle variant |
| **F18** | Census arithmetic: 83 vs the ledger's 98, over a corpus containing non-sites | **FOLDED** | §Goal + §Integration ledger — 179/36 reproduced by one command; 98 named as the occurrence remainder; the **walker's site count** named as the migration denominator so comment mentions can never be driven into the allowlist |
| **F19** | G1 is scheduled at L0-gate but depends on L1 | **FOLDED** | §Implementation plan — G1 moves to **L1-gate**; L0-gate keeps G4 and G2, both of which have real red states at L0 |
| **F20** | L2-gate reds (or goes vacuous) on the grandfathered corpus | **FOLDED** | §Decision 6 — registry rows carry `Live` / `Pending(rung)`; check 3 (Live ⇒ ≥1 emitter), check 3b (Pending ⇒ 0 emitters), check 3c (`Pending == 0`, armed at L8c). L2 commits alone; a `Pending` row cannot rot silently |
| **F21** | `LANE_ARRAY_LEN = 0` vs `LANES[i]` indexing in Algorithms B | **FOLDED** | §Decision 3 + §Algorithms B — the claim scan is written over `LANES.iter()`; zero-length is zero iterations |
| **F22** | `dsp!`'s described form does not borrow-check | **FOLDED** | §Decision 13 — the `DspBuf<N>` by-value-temporary form is written out, with the end-of-statement lifetime argument and the 256-byte cost in the trade-off |
| **F23** | `Lane`'s SAFETY block does not cover `read_cached` / `write_cached` | **FOLDED** | §Data structures SAFETY clauses **1e** and **1f** — single-role ownership plus the staleness argument in both directions |
| **F24** | `W0102` is unbudgeted: ~16 000 sink-generated records/s during a drop storm | **FOLDED** | §Decision 5 + §Algorithms C — **one aggregated `W0102` per drain** (125/s) carrying `lanes_affected`/`records`/`bytes`/`SATURATED`; per-lane detail moves to the polled census |
| **F25** | `STAGE_BYTES`'s backing store unspecified | **FOLDED** | §Data structures — `STAGE`, `SITE_DICT`, `SINK_OUT` are `.bss` statics, counted in Decision 3's budget matrix, with the "no `Vec`/`Box` in a *signature*" narrowness called out |
| **F26** | Zero-alloc test 3 proves only the steady state (TLS dtor registration) | **FOLDED** | §Metrics test 3 — three legs (steady 0, first-emit ≤ 1 with the number recorded, monotonicity over 1000 emits) and a new perf-table row |

### Refuted, with the evidence

| # | Claim | Refutation |
|---|---|---|
| **F5 (count)** | "28 occurrences of `boyko-B1802`, every one inside a `/// # Panics` doc comment" | **Partially refuted.** `crates/boyko_ecs/src/ecs/core/app/app.rs` contains **24** occurrences: 18 in doc comments, 1 panic-message string (`:867`), 5 `#[should_panic(expected=)]` inside the in-`src` `#[cfg(test)]` module (`:898`-`:939`). The *finding* is correct and folded; the *count* is not, and the fix is specified against the measured shape (the `#[cfg(test)]` region also has to be stripped, which the review's framing did not surface) |
| **F12 (part)** | "`eprintln!` currently takes `stderr()`'s own lock, so the replacement … is a regression against M24's own concern" | **Accepted as to `debug.rs`** — and the conclusion is stronger than the review's: the whole edit is withdrawn, so the concern cannot arise. But the *general* claim that `stderr()`'s lock made v2 safe is not why `report!` needed fixing: `report!` writes **stdout**, where the hazard is the `LineWriter`, not the lock (F17). The two are separate defects and are fixed separately |
| **F16 (framing)** | "`boyko_log`'s statics are almost certainly correct but Principle 0's named exceptions do not cover them" | **Accepted, and the exception is claimed** — but not as a formality. The argument is *load-bearing*: it is the same argument that forbids `CONTROL` from being an ECS column (§Refused), and it is what makes the crate usable from a driver callback and a panic hook. Recording it as a bare exception would lose that |

**One finding is deliberately NOT folded as written:** the review's open question 5 asks "`LogLine` must be `Copy` and its size must divide 64 KiB. Which size — 8 or 16?" **16**, with the fields listed in §Data structures. 8 bytes cannot carry `start` + `len` + `code` + `level` + `target` without dropping the sequence number that Decision 26's reader cursor needs.

---

## Scope-extension disposition (games as a first-class audience)

### What the extension CHANGED in the engine design, and the argument for each

| # | v2 element | Change | Argument |
|---|---|---|---|
| **X1** | `CEILINGS: [AtomicU8; 256]`, one byte = one level | Renamed `CONTROL`; the byte is packed `[0..2] level ‖ [3..6] sample shift ‖ [7] sync route` | Three runtime knobs delivered in **the register already loaded** — no second array, no second cache line, one extra `and`. Decision 14's single-owner property is preserved: still one byte, one authority, no mirror. `.bss`-zero still means fully off. **Gated**: `log_disabled_runtime` must stay NOT RESOLVED against the v2 shape (G10d), else the packing is reverted |
| **X2** | Downstream target band 96..=255 | Re-cut: source 96..=223, **dynamic 224..=255** | The dynamic band is the concrete answer to "declared from data / a mod / a script". 32 slots is a deliberate cap (open question 8). Decision 15's *mechanism* is unchanged |
| **X3** | `RatePolicy::EveryN(u16)`, arbitrary `n` | `n` must be a power of two (`const _` assert); `count & (n-1)` replaces `count % n` | v2's form mis-samples across the `u32` wrap (~12 h at 100 K·s⁻¹) — invisible in a 300-frame bench, wrong in a session. The fix is *also* cheaper. Strictly better on both axes |
| **X4** | `dropped` / `dropped_bytes`: plain `fetch_add` | **Saturating**; census renders `SATURATED` | With no drain running the counter wraps in ~65 s and then reports a small, credible, wrong number. `AtomicU64` was rejected: an 8-byte RMW that still would not make the *reported* value unambiguous |
| **X5** | `LogSite` fields | `+ fields: &'static [&'static str]`, `+ prefix` | Structured/telemetry output needs names, and a game needs its own code prefix. `LogSite` is `&'static`, cold, and never touched on the emission path — free everywhere it matters |
| **X6** | `Lane` line 2 | `+ sampled_out: AtomicU32` (pad 52 → 48) | A sampled-out record is **not** a loss; counting it into `dropped` would corrupt the drop count in the other direction, and the `emitted == drained + dropped + sampled_out` property depends on the separation |
| **X7** | `FileSink { path, max_bytes }`, stop-at-cap | `+ Rotation { max_bytes, keep }`; **`Rotation::NONE` remains the engine default** | An hours-long session needs rotation; a bench must not silently discard its own beginning. Both, selected by profile. Every rotated file re-emits anchor + dictionary so it decodes standalone |
| **X8** | Sinks "boot-published, never mutated after boot" | Kind stays boot-fixed; **state / filter / floor become runtime byte stores**; open/close go through a 16-entry `SINK_REQ` under `OUT_LOCK` | A game toggles capture from a console with no restart. All I/O stays on the sink thread (G13b proves zero allocations on the requesting thread). A channel was rejected: an allocation and usually a `Mutex` |
| **X9** | `SINK_STATE ∈ {NotBooted, Running, Manual, Exited}` | `+ Exiting, CrashDraining` | The crash path needs a state meaning "the consumer role has been transferred". The transfer is a CAS out of three provably-quiescent states, so it adds no race and no wait |
| **X10** | Census statuses `MEASURED / UNPROVEN / UNPROVEN(lossy)` | `+ UNPROVEN(sampled)`, `+ UNPROVEN(unsunk)`, `+ dropped=SATURATED(...)` | The second audience creates three new ways to build a vacuous gate: sample a target and read the count as a total; enable a target no sink accepts and read the silence as clean; read a saturated counter as a number. Each gets a status; `unsunk` also gets `W0111` |
| **X11** | `CensusPolicy` implicit (`OnFlush`) | Explicit `OnFlush` (dev default, unchanged) `/ OnShutdown / Interval(secs)` | `OnFlush` in a game that flushes per frame is a per-frame census line. The engine default does not move |
| **X12** | Perf target rows | 6 rows added; **no existing row weakened**; the two measured rows keep their numbers **and gain a gate that can revert the rung threatening them** | The extension is not permitted to buy flexibility with the engine's measured budget. Where a new mechanism touches a measured path, the gate's failure disposition is "revert or feature-gate the extension", never "raise the target" |

### What the extension deliberately did NOT change

`report!` (text, stdout, synchronous, byte-frozen; explicitly not a game-facing API) · the `[vk-validation]` channel (**not edited at all** — v2's one edit is withdrawn too) · Decision 7 (`Warn`/`Error` MUST carry a code — no exception for games, mods or scripts) · Decision 13's structural re-entrancy exclusion (`LogPod::fmt_pod` runs on the sink, asserted by test 24) · Decision 12 (no handle, bounded waits, detach-not-join) · Decision 1's deferred-format core and the 20 B packed header (session/frame/tick correlation is an **anchor record**, not a per-record field) · the eight registry checks and their corpus rules · Decision 11's `rdtsc` and its uncontrolled `W0101` · Decision 3's off-build · the `.bss`/`Off == 0` regime · the SPSC lane and its SAFETY clauses · gates G1-G7 and their limits · every migration-ledger disposition.

### Refused, with the reason recorded so it is not re-derived

| Ask | Reason |
|---|---|
| **Network sink inside `boyko_log`** | Blocking, retries, back-pressure — the three properties this crate refuses. `CallbackSink` on the sink thread is the seam; the game owns the policy because only the game knows whether stalling or dropping is worse for it. G13c demonstrates the stall cost rather than leaving it as prose |
| **Cross-process shared-memory ring** | The record carries a process-local `*const LogSite`. Supported instead: per-process files, one `SessionId`, `logdec --merge`, with the clock-agreement bound **printed** |
| **Gameplay decisions on log counters** | Lower bounds under drop, schedule-dependent, non-deterministic across machines ⇒ breaks replay. Display and telemetry are supported (`LogCensus.lossy` is the bit that keeps a UI honest); gameplay counters belong in the game's own components — Principle 0's answer |
| **Data/script-defined *codes*** | A code is a promise of a documented page; a data-defined code cannot have one. Supported instead: one class code per game subsystem, plus dynamic *targets* |
| **`CONTROL` as an ECS column with `EnableTag`** | Dependency cycle · no `World` at boot, in a panic hook or in a driver callback · ~10-30 ns per emit against a 1-instruction budget. The rule's substance (capability structural, state a bit) is applied at the layer that can afford it, and the exact cost of the refusal (no `Query`, no change detection) is stated with `CONTROL_EPOCH` as the mitigation |
| **Per-entity log storage in `ComponentPool`** | Logging is not per-entity data; `UnitId`'s two-level addressing buys nothing for a byte ring |
| **`Box<dyn Sink>` plugins** | `extern "C" fn(&FormattedRecord, *mut ())` + ctx crosses a dylib boundary with no vtable and no allocation |
| **A second sink thread for the binary sink** | Two consumers on one lane set is what the `Lane` SAFETY block forbids. Sinks fan out **inside** one drain, so text + binary + crash cost one pass |
| **Growing the ring to answer "as much data as possible"** | Enlarging the ring moves the loss point; it does not raise the throughput ceiling, which is `core::fmt` on the sink. The answer is to **not format** (`BinarySink`) — and that claim ships with a revert clause (G12c) rather than as an assertion |

### The one thing this plan says plainly is a bad idea

**Do not make the logger a substitute for a missing source.** This is not a hypothetical: sync-validation is **dead on this machine** — a genuine missed barrier produced 19 messages (the baseline), zero `SYNC-HAZARD`, and a byte-identical golden, twice. A logger is a transport. It changes where a message goes and has no opinion on whether the message exists. Routing a dead channel through a prettier pipe makes the deadness *harder* to see, which is why v1's migration is withdrawn and why the census reports `UNPROVEN`, never `clean`.

The same reasoning bounds the game-facing ask. A logger cannot tell a game why its frame hitched if nothing measures frames; it cannot tell a player's support agent what went wrong in a crash that did not unwind; and it cannot make a sampled capture representative. Each of those is written into a gate's "cannot claim" column rather than left for a reader to discover at the moment they need it to be true.