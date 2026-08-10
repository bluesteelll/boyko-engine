# Profiling — emission ABI, transport, and the frame-major store

<!-- CONTRACT
provides: profiling/emission-abi
provides: profiling/store-and-fold
assumes: substrate/mute-leaf-rule
assumes: substrate/clock-source
assumes: substrate/lane-registry
assumes: substrate/lane-write-sites
assumes: substrate/loss-vocabulary
assumes: substrate/loss-fold
assumes: substrate/never-freed-storage
assumes: substrate/section-report
assumes: seam/build-axis
assumes: seam/free-when-off
assumes: seam/joint-cost
assumes: seam/lifecycle-order
assumes: profiling/budgets-and-invariants
-->

*Carved from `docs/PROFILING-SYSTEM-PLAN.md` (rev 4) — §D1, D2, D3, D3a, D6, D8, D9, D15, D16, D19, D21, D24; the emission/store half of §"Data structures" and §"Public API"; algorithms A1, A2, A7, A9; and the whole §"Multithreading model". Diff against the monolith until it is retired.*

Two halves live here, on opposite sides of the crate graph: **the emission ABI in `boyko_diag::profiling_abi`** (zero dependencies, allocates nothing, names no `World`) and **the durable store in `boyko_ecs::ecs::core::profiling`** (a `Resource` on kernel VM-native storage). The seam between them is one `u16` and 24 bytes.

---

# Part 1 — Emission (`boyko_diag::profiling_abi`)

## D1 — Emission: two gates, no allocation, no lock, one 24 B store

`zone!(HANDLE)` expands (feature on, site tier ≤ `GLOBAL_TIER`) to `let _z = ZoneGuard::open(&HANDLE);` — one `Acquire` load of a `CachePadded` global `ARM_MASK`, one statically-predicted-not-taken `bt`, one `rdtsc`; on `Drop`, one branch, one `rdtsc`, one **24 B** store (one 16 B + one 8 B), one `Release` cursor store. **The two off-axes do NOT expand alike, and this sentence conflated them until now.** With the FEATURE off, `#[cfg]` deletes the macro definition before name resolution, so there is no expansion and the argument is never named. **Above the tier ceiling the macro still expands and still NAMES its argument** — twice over: the gate reads `const { $h::TIER as u8 <= GLOBAL_TIER as u8 }` from the `mod` companion, and the guard body is `ZoneGuard::open(&$h)`. The `const false` then deletes the *codegen*, not the tokens. Zero instructions either way; an undeclared identifier is `E0425` only in the second case.

**Why 24 B and not rev 3's 16 B (B1), and what it costs.** Rev 3 used one 16 B record for all three kinds, with `begin` meaning *TSC at open* for a `Span`, *the value* for a `Counter`/`Gauge`, and *the high 32 bits of `dur`* for an `Extension`. The fold reads that field **before** the kind dispatch — for the live-frame cut and for the frame walk — so a counter's payload was consumed as a timestamp: a typical count (10³-10⁹) sits far below the cut (a TSC ~10¹³-10¹⁷) and every counter sample landed in `drops.late`, while a large one (a byte count, a handle) exceeded the cut and truncated the whole region's fold for that frame. The same defect hit the `Extension` record, which the review did not name: its dur-high-bits were also read as a TSC, so a span longer than `u32::MAX` ticks — *the hitch most worth recording* — silently lost its high word **and** was mis-attributed. One record shape for three meanings was the root cause, not the counter kind.

The record therefore gains a field that means "when" for **every** kind, and the payload gets its own 64 bits:

| Field | Width | Span | Counter | Gauge |
|---|---|---|---|---|
| `stamp` | `u64` | TSC at **open** | TSC at the emit call | TSC at the emit call |
| `value` | `u64` | duration in TSC ticks | the increment | the level |
| `zone` / `flags` | `u16` / `u16` | id + kind + gpu-origin bit | | |

`stamp` is the **only** field attribution reads, and it is read identically for all three kinds — so no reordering of the fold can reintroduce the defect. `value` at 64 bits **deletes the saturation path entirely**: no `Extension` sample, no `[2] saturated` flag, no `#[cold]` second store, and one compare-and-branch fewer in `Drop`. Net instruction count on the emission path is **one lower** than rev 3 (−1 compare, −1 branch, +1 store µop).

**The cache claim, re-derived against the new shape rather than inherited.** A 24 B record is 2.67 per 64 B line, not 4 — **0.375 line touches per sample instead of 0.25**, and with a 64 B-aligned ring base 2 of every 8 records straddle a line boundary (offsets 48 and 56), so those two stores split. This is the honest cost of the fix and it is not hidden: the alternative, a 32 B record, is 2 per line (0.5 touches/sample) and doubles the ring's bytes, so 24 B dominates it on both counts. The **≤ 12 ns budget is retained** — one fewer instruction against +0.125 line touches on a monotone, write-allocated, hardware-prefetched cursor whose stores retire into the store buffer — and it is *re-gated*, not re-asserted: `zone_cost`'s +25 % threshold is taken against a baseline measured for this shape. Nothing is built yet, so no committed baseline is invalidated by the change.

**Why the mask load is `Acquire`, not `Relaxed` (F11).** `ARM_MASK` gates the lane `buf` pointer, which is published `Release` at first arm. Rev 2 loaded the mask `Relaxed` and then loaded `buf` `Acquire`; the abstract machine then permits observing a set mask together with a stale null `buf`, and the hot path stores 16 B through it with no null test. **On x86-64 an `Acquire` load of an aligned word is the same single `mov` as a `Relaxed` one** — there is no fence, so the correction costs zero instructions. Rev 2's stated reason for `Relaxed` ("`Acquire` would forbid nothing while costing a fence off x86") had the cost backwards and the ordering obligation missed. The publication order is pinned in `arm()`: **slab → every `buf` (`Release`) → `ARM_MASK` (`Release`), in that order, always.**

**Why this shape.** NanoLog/Quill measure 7-9 ns with exactly it; spdlog measures 242 ns with the same asynchrony but caller-side formatting — the delta is entirely "do no work at the call site". At 400 zones × 60 Hz, 12 ns costs 0.03 % of a frame; 250 ns would cost 6 ms/s. The gate order (`const` tier ceiling `&&` runtime mask) is `log!`'s verified expansion: short-circuit `&&` over a `const` guarantees the arm and its operands vanish.

**Rejected.** `tracing` / `log` (third-party; `tracing`'s disabled check is a static callsite + two atomic loads, and its layers are `Box<dyn Layer>`). `Instant::now()` — **20-30 ns per pair** (`crates/bench_bevy_vs_boyko/benches/profile_spawn.rs:229-230`), which alone is 2× the whole open+close budget. `thread_local!` rings (TLS destructors at thread exit — the canonical lock-free-logger bug; the engine already has a pool-owned lane index). An `AtomicBool::swap` once-latch in a reader (`crates/boyko_render/src/render_path_config.rs:311-313` executes an RMW on a shared line every frame forever once its condition holds).

**Trade-off.** A `mem::forget`ed guard loses its sample silently in release; `ZoneGuard` is `#[must_use]` and a debug-only TLS depth counter (D3a) catches it in debug.

**Correction, not a trade-off — the zero-instruction fix (F8) costs nothing on this axis, and this line asserted the opposite for four revisions.** The TIER fold deletes codegen, not tokens. The expansion names the identifier twice — `const { $h::TIER as u8 <= GLOBAL_TIER as u8 }` in the gate and `ZoneGuard::open(&$h)` in the body — and name resolution runs on both regardless of which way the `const` folds. *(The ~12 KiB of dead `.bss` D21 books for folded `ZoneHandle` statics is a true and adjacent fact, but it licenses nothing about naming: a `pub static` is emitted because `declare_zone!` declared it, not because anything reads it. An earlier revision of this line inferred one from the other.)* So a typo'd zone identifier at a `Deep` site is a hard `E0425` in **every** feature-on profile, retail included — **rung 14's `Deep` leg is not "the one that catches it"; every feature-on leg does.** The only leg that cannot see it is the FEATURE-off one, where `#[cfg]` deletes the macro definition before name resolution, and that leg is G1(a)'s subject, not this one's.

**The two gates are the two axes of S13** (`seam/free-when-off`). Gate 0 is the **compile-time ceiling** and is the only one that reaches zero: a `const false` deletes the arm and its operands, leaving no branch, no symbol and no `.bss` row. Gate 1 is the **runtime flag** (`ARM_MASK`), default 0 because `.bss` is zero, and it is what lets a shipped binary be asked for a measurement after the fact. **Gate 1 cannot be driven to zero cost** — one `.bss` load plus one predicted branch per surviving site, in every frame, forever — and this plan does not claim otherwise.

## D2 — The lane registry is `boyko_diag`'s, not this plan's; the lane is resolved ONCE per thread

**Rev 3 owned a lane taxonomy. Rev 4 does not — it consumes one (S3, `substrate/lane-registry`).** `boyko_diag::lane` is the single registry for both diagnostics subsystems, because two topologies means two lane numbers for one thread, and then no reader can place a log line inside the zone it happened in. The profiler's remaining stake is that the taxonomy match the ACTUAL thread topology: the engine has no present thread and no asset thread, and the real hazard is that **the host thread is `UNATTACHED` outside `install`** (`crates/boyko_threadpool/src/tls.rs:29`) and therefore collapses onto lane 0 — worker 0's lane — precisely while it drives the post-present GPU readback. That property is preserved by the shared registry:

```
lane 0..63    workers        (dense pool worker id — MAX_WORKERS = 64, thread_pool.rs:49)
lane 64       LANE_DISPATCHER (host thread INSIDE ThreadPool::install)
lane 65       LANE_HOST       (host thread OUTSIDE install; claimed by the runner at boot)
lane 66..     spares, claim_lane() / release_lane(), #[cold]
lane 0xFFFF   LANE_UNCLAIMED  (emission is refused and counted)
LANE_COUNT = 80 in EVERY profile — no profile axis (Q1 RESOLVED)
```

`LANE_COUNT` is a **max, not a sum**: 64 is a hard const, plus dispatcher, host and 14 claimable spares (7× the measured non-pool thread count in this engine). Rev 3's 68 is superseded. **Q1 is RESOLVED and it did not pick one of the tabled numbers:** it deleted the profile axis, because `LANE_COUNT` was made per-profile while the quantity it indexes — `MAX_WORKERS = 64` — is unconditional (`substrate/02-LANE.md`). This plan consumed the shipping 32 in four places; all four are corrected below, and the correction **raises the shipping total past the ≤ 1 MiB headline** — the consequence is stated at the sizing table rather than absorbed.

**Rev 2 specified lane resolution twice, incompatibly** (F12): A1 step 4 said "lane from a TLS `Cell<u16>`, one load", rev 2's D2 said "worker id `< 64` → that lane; `WORKER_ID_DISPATCHER` → 64; else the TLS-claimed lane; else drop" — different mechanisms with different costs — and nothing in the integration table set the TLS for workers, so *every worker would have resolved "unclaimed" and dropped*.

**Single specification, now in the shared crate.** `boyko_diag::lane::LANE: Cell<u16>` defaults to `LANE_UNCLAIMED` and carries **no `Drop` guard** — TLS destructors at thread exit are the canonical lock-free-logger bug and are refused here as they were in rev 3 (D1's rejected list). It is **written once per thread**; the write sites are `substrate/lane-write-sites`' to enumerate (there are **three**, not the two the seam record stated, the third being `InstallGuard::drop` on the unwinding path), and this plan's summary of them is:

| Site | Value | Crate |
|---|---|---|
| `worker_main` entry, beside `set_current_worker_id` | the pool's dense worker id | `boyko_threadpool` |
| `ThreadPool::install` entry (and restore on exit) | `LANE_DISPATCHER` | `boyko_threadpool` |
| `boyko_diag::lane::claim_lane()` from the runner at boot | `LANE_HOST`, else a spare, else `None` | `boyko_app::runner` |

Emission is then **one TLS load + one compare against `LANE_UNCLAIMED`** — the same cost as rev 3, and now **one** TLS slot for both subsystems instead of two. The three-branch worker-id derivation is *initialisation*, performed once, not per sample. One OS thread may hold two lane identities over its life (dispatcher inside `install`, host outside); this is sound because the thread is serial, so each lane still has exactly one writer, and samples carry absolute TSC so the timeline joins without a clock epoch.

**Cost of sharing, stated.** The claim scan no longer spreads by thread-id hash, so concurrent claimants of the 14 spares can convoy on the first free slot — bounded at 14 CAS attempts on a `#[cold]` path taken once per thread. A thread that never calls `release_lane()` holds its spare for the process: bounded, counted as `lanes_leaked`, printed in the census. **Benefit, stated honestly:** the shared registry buys *agreement*, not speed — G7 gains a join clause (a `warn!` and a zone emitted on the same worker must report the **same integer**), and that clause is the whole reason the registry moved.

**Rejected.** An MPSC fallback lane (a second ring type, a second fold path, for zero threads). Widening `current_worker_id_or_dispatcher_lane` (it is the event system's contract; changing its `UNATTACHED → 0` sentinel would move event traffic — `tls.rs:69-78`).

## D3 — The CPU clock is `boyko_diag::clock`, shared with the logger; this plan owns only its consumption

**Rev 3 owned a clock. Rev 4 does not (S4, `substrate/clock-source`).** `boyko_diag::clock` is the single owner: `ticks()`, `ticks_per_ns()`, `clock_epoch()`, `calibrate()`, `note_forward_jump()`, `invariant_tsc()`, `session_id()`. Both subsystems store **raw ticks** and both read the scale and the epoch from it. `calibrate()` is idempotent and is called by whichever of `boyko_log::enable` / `Profiler::arm` runs first — **the enable path, not boot** (S13; with both subsystems off the clock is never calibrated and never read).

The mechanism is rev 3's, moved: `ticks()` → `_rdtsc()` when `CPUID.80000007H:EDX[8]` (invariant TSC) is set; a QPC-derived tick otherwise, with `boyko-W9207` (the single invariant-TSC code — the logging plan's `W0101` is deleted in favour of it) and a raised quantum. `calibrate()` runs 16 probe pairs over a bounded `CALIB_WINDOW_MS = 20` window, discards probes whose `(rdtsc, Instant)` disagreement exceeds `1.5 × min_disagreement` (Tracy's rejection sampler), and publishes `ticks_per_ns` with **`calib_cv`** and `calib_rejected`. `Profiler::arm` remains a setup call (`debug_assert!(!is_in_system_run())`).

**Why CV and not the worst probe:** peak-to-peak grows with `n` and cannot reproduce itself; attaching the worst-of-N probe to every printed nanosecond would be the same defect with the opposite verdict.

**What sharing actually buys — agreement, not time.** The boot saving is roughly one `cpuid`, not 20 ms, because the calibration still has to happen once; **the plan says so rather than claiming a speedup.** What it buys is that a suspend/resume cannot produce a profiler window quarantined as `ClockEpochBreak` while, in the same seconds, log lines carry wall times wrong by the suspend duration with no marker — two artifacts that disagree, neither of which says why.

**Clock epoch breaks (X22, session-scale).** A game session is hours; suspend/resume and some power transitions move the TSC. The fold compares the frame's elapsed ticks against `MAX_PLAUSIBLE_FRAME_TICKS`; on violation it calls `boyko_diag::clock::note_forward_jump()` — which bumps the shared `clock_epoch` and raises `DiagFlag::ClockEpochBreak` — discards the in-flight window, counts `clock_epoch_breaks`, emits `W9216` once per break and re-runs `#[cold] calibrate()`. **No sample crosses a break, `resolve` refuses any leg whose `clock_epoch` differs from its partner's** (C-V), and the logger's `RecordHeader` carries the same `clock_epoch` so a straddling log record is legible beside the quarantined window. The joint RED is one injected forward jump asserted on **both** artifacts (S4).

**Trade-off.** `rdtsc` is not serializing; the OoO engine may move instructions across a bracket. Consequence, printed as a field: the CPU channel's **quantum** is the measured `__cpu_null` median, and no span shorter than it is reported as a number. (Unlike its GPU sibling, `__cpu_null` is *not* measured to be zero — see D11a in `profiling/03-STATISTICS.md`.)

### D3a — `depth` is debug-only and lives in TLS, not in the lane

Rev 1 had `depth: u16` as a plain field inside a `static` — mutating it through `&'static` without `UnsafeCell` is UB, and no hot-path step incremented it. Removed. Nesting is reconstructed at fold (a region is single-writer, so its samples form an exact stack). Forgotten-guard detection is `#[cfg(debug_assertions)] OPEN_DEPTH: Cell<u16>` in TLS — zero release cost, no UB — and the fold's `debug_assert!(OPEN_DEPTH == 0)` becomes meaningful. `capacity` is a `const`, not a field.

## D6 — Zone identity: a dense `u16` minted once, one registry, no strings on the emission path, exhaustion NON-terminal

```rust
declare_zone!(VB_EARLY_RASTER,
    name = "vb_early_raster", channel = Channel::GpuPass, kind = ZoneKind::Span,
    stage = GpuStage::BottomOfPipe, group = PartitionGroup::VbRun,
    scope = Scope::Render, tier = ZoneTier::Dev);
```

expands to **two** items under one identifier, and the second one is load-bearing rather than
cosmetic:

```rust
pub static VB_EARLY_RASTER: ZoneHandle { desc: &'static ZoneDesc, id: AtomicU16 }
#[allow(non_snake_case)]
pub mod  VB_EARLY_RASTER { use super::*; pub const TIER: ZoneTier = /* the declared tier */; }
```

**Why the tier is duplicated into a `mod` companion instead of being read off the handle.**
`zone!`'s first gate is a `const` block, and a `const` block **cannot read through
`VB_EARLY_RASTER`**: the handle carries an `AtomicU16`, so `const { VB_EARLY_RASTER.desc.tier … }`
is `error[E0080]: constant accesses mutable global memory` — **measured on this box**, rustc
1.95.0, `--edition 2024`, and it fails identically whether the comparison would fold true or
false. A `mod` with the same name is legal because a static lives in the VALUE namespace and a
module in the TYPE namespace, so the two coexist; a **unit struct does not work** here — it
collides with the static in the value namespace (`E0428`). The alternative, dropping the
`AtomicU16` from `ZoneHandle`, also compiles but relocates the mint path (D6) and is the larger
change, so it is not taken.

⚠️ **The `use super::*;` is load-bearing, not tidiness, and the first probe of this design could
not have found that out.** A `macro_rules!`-emitted `mod` is a **fresh scope**: it inherits none
of the caller's imports, and the tier it carries is the **caller's own path token**
(`ZoneTier::Dev`) relocated into that scope. Without the glob, every `declare_zone!` in the engine
fails — measured with a MACRO probe, rustc 1.95.0, `--edition 2024`:
`error[E0433]: cannot find type ``ZoneTier`` in this scope`. **`$crate`-qualifying the const's
type annotation — the reflex any macro author reaches for first — does not fix it**, because what
fails to resolve is the caller's `$tier` expression, not the annotation; the probe above was
already `$crate`-qualified and still failed.

*The earlier "measured on this box … compiles" claim in this section was true of a **hand-written**
module beside a hand-written static, which resolves its paths in whatever the prober wrote around
it — and is therefore silent about the one thing this design newly depends on. Proving the wrong
thing is the failure mode; the fix is to probe the FORM the design uses, not a stand-in for it.*

*Do not substitute `pub struct $name {}` for the module: it trips `non_camel_case_types`, which
`#[allow(non_snake_case)]` does not cover and which `cargo clippy --all-targets -- -D warnings`
turns into a hard error at every zone declaration.*

**The typo-catching property survives the companion**, which is the only reason it is acceptable:
`zone!(VB_EARLY_RASTRE)` fails with `E0425` **and** `E0433` (the value and the module both fail to
resolve), measured with the same toolchain.

**Which partition a site mints from is a property of the DECLARING CRATE, not of the macro it used (B3).** Rev 3 keyed the partition on the macro — `declare_zone!` → engine, `register_zone` → dynamic — and then recommended `declare_zone!` as the game path ("X1 needs no new mechanism at all"). Those two statements together put the recommended game path *inside* the partition the design exists to protect: a plugin with 3000 static zones would exhaust the engine id range and a plugin looping a static zone would overflow the engine ring — the exact two failures G11 and G20 are written to exclude, while both gates passed, because both exercised only `register_zone`. That is the vacuous-gate shape: the gate's input class excludes the defect.

The key becomes the crate, stated once at its root and **not defaultable**:

```rust
// Once per crate that declares any zone. No default: a crate that declares a zone
// without this line does not compile (unresolved `crate::__BOYKO_ZONE_PARTITION`).
boyko_diag::profiling_partition!(Engine);   // engine crates
boyko_diag::profiling_partition!(User);     // games, plugins, mods, tools, benches

// `Engine` is not merely a convention — the expansion const-asserts the caller's identity:
//   const _: () = assert!(boyko_diag::is_engine_package(env!("CARGO_PKG_NAME")),
//                         "profiling_partition!(Engine) is for engine crates; use (User)");
// `env!` expands at the INVOCATION site, so the name is the invoking crate's package.
// `ENGINE_PACKAGES` is a const list in boyko_diag; a tidy test pins it against the
// workspace members that ship inside a game binary. `boyko_demo` is a GAME and is
// deliberately NOT in it — a name-prefix rule would have swept it into the engine.
```

`declare_zone!` then reads `crate::__BOYKO_ZONE_PARTITION`, a compile-time constant, for **both** the id counter and the ring region. Neither is a per-site choice, so a game cannot mint one engine zone by accident, and a downstream crate cannot mint any: `profiling_partition!(Engine)` fails to compile outside the pinned package list.

**Minting — a total order over real values (F9), now over the crate's partition counter.**

```
   P = crate::__BOYKO_ZONE_PARTITION        // compile-time: Engine | User
   (NEXT, BASE, LIMIT) = match P {
        Engine => (ENGINE_ID_NEXT, 0,                 ENGINE_ZONE_SLOTS),
        User   => (USER_ID_NEXT,   ENGINE_ZONE_SLOTS, ENGINE_ZONE_SLOTS + armed_user_budget),
   }
1. CAS handle.id: UNASSIGNED -> RESERVED        (Acquire on success, Relaxed on failure)
      loser -> #[cold] spin until id != RESERVED; return it
2. n = NEXT.fetch_add(1, Relaxed)               // n now exists
3. if BASE + n >= LIMIT {
       NEXT.fetch_sub(1, Relaxed);              // monotone reservation restored, no id leaked
       handle.id.store(DISABLED, Release);
       drops.zones_refused += 1;                // W9201 (Engine) / W9210 (User), once each
       return DISABLED
   }
4. REGISTRY[BASE + n].store(desc_ptr, Release)  // the desc is published FIRST
5. handle.id.store(BASE + n, Release)           // the id is published LAST
```

✅ **SHIPPED at rung 10 — and the split did not exist before it.** MEASURED at rung 10's opening:
`crates/boyko_diag/src/profiling_abi.rs` had ONE counter, `NEXT_SLOT`, serving both partitions over
one 4096-slot range. `Region` picked the ring but not the id range, so everything this section says
about separate counters was true of the design and not of the code — a `User` crate's static zone
minted an ENGINE id, which is precisely the defect `G11`'s `[B3-fix]` was rewritten to catch and
which its RED now reproduces (id **1** for a `profiling_partition!(User)` crate's zone). The
counters are now `ENGINE_ID_NEXT` and `USER_ID_NEXT`, `REGISTRY` spans
`ZONE_ID_SPACE = ENGINE_ZONE_SLOTS + MAX_USER_BUDGET`, and `mint_cold` routes on
`handle.desc.region` — the descriptor field the macro already filled from the declaring crate.

**`USER_ID_NEXT` is one counter for both user authoring paths** — a game's static `declare_zone!` and its dynamic `register_zone` draw from the same range and the same budget, because they are the same traffic from the id space's point of view. Rev 3's `DYN_ID_NEXT` is renamed to it, and `ProfilerConfig::dyn_zone_budget` becomes `user_zone_budget`.

Rev 1's reserve-then-CAS leaked a counter value per lost race, making the id space sparse and firing exhaustion early; that fix is retained, now in an executable order.

**Ordering, specified ONCE (F10).** Rev 2 gave `AcqRel`/`Acquire` in the multithreading table and `Relaxed` in D6/A1. The single truth:

- `handle.id` — store `Release` (step 5), **load `Relaxed` on the emission path**. Sound because *the emitter never dereferences a desc*: it stores a bare `u16` into the sample.
- `REGISTRY[n]` — store `Release` (step 4), load `Acquire` at fold / report. This is the **only** desc edge. A fold that reads `REGISTRY[n]`'s stored value with `Acquire` synchronises-with the registrant's `Release`, and every byte of the desc was written before it. That holds whether or not the emitter is the registrant, which is what makes the emitter's `Relaxed` id load safe.

**One registry, one truth.** `static REGISTRY: [AtomicPtr<ZoneDesc>; ZONE_ID_SPACE]`. The `Profiler` Resource holds **no desc mirror** (rev 1 had two); the window reducer reads `REGISTRY`.

System zones are pre-registered at `ScheduleBuilder::try_build` **when `GLOBAL_TIER >= Dev`**, so their emission path never takes the registration branch (the branch is still emitted; it is statically predicted not-taken and never taken).

**Exhaustion is NON-terminal (F5, C-III).** Rev 2 mirrored `query_type_registry.rs:124-144`'s terminal `E9201`. That precedent does not transfer: a query-shape registry is answering a *semantic* question, where a missing entry is a wrong answer; a zone registry is answering a *measurement* question, where a missing entry is a missing measurement. And the arithmetic makes the terminal form dangerous: `MAX_SYSTEMS_PER_SCHEDULE = 1024` (`crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:70`), an `App` runs at least `Startup` + `Fixed` + `Main`, per-system minting was unconditional, and the feature is default-on — **so a legal app that never asked to profile could panic at build time.** Exhaustion now yields `ZONE_ID_EXHAUSTED`, increments the substrate's `LossClass::Refused` cell, raises `DiagFlag::ZoneRegistryExhausted` and is emitted as `W9201` (engine) or `W9210` (user) once by the profiling fold. *(Landed at rung 3 with those names: `ZoneId::DISABLED` and a `zones_refused` field are the rev-4 spellings; the counter is the shared loss cell, because the substrate is mute and the count has to exist below the emitter.)* `W9208` still fires once at 90 % engine occupancy. G11 is the gate that makes the counter non-vacuous.

**Id space, sized against the kernel's own cap, and keyed on the crate partition:**

| Range | `dev` / `editor` | `shipping` / `shipping-min` | Contents | Counter |
|---|---|---|---|---|
| `0 .. ENGINE_ZONE_SLOTS` | 4096 | 256 | zones declared by crates whose root says `profiling_partition!(Engine)`, including ≤ 1024 systems × up to 3 schedules | `ENGINE_ID_NEXT` |
| `ENGINE_ZONE_SLOTS .. + user_zone_budget` | `≤ MAX_USER_BUDGET = 3072`, default 256 | `≤ MAX_USER_BUDGET = 512`, default 0 | **every** zone declared by a `profiling_partition!(User)` crate — static `declare_zone!` and dynamic `register_zone` alike (D19) | `USER_ID_NEXT` |

`ZONE_ID_SPACE = ENGINE_ZONE_SLOTS + MAX_USER_BUDGET` — a compile-time const, so `REGISTRY` is `.bss`: 7168 × 8 B = **56 KiB** in `dev`, 768 × 8 B = **6 KiB** in `shipping`. `MAX_USER_BUDGET` is **profile-dependent** (3072 dev / 512 shipping) for the reason M10 names: rev 3 sized it by a single constant, so a retail build carried 208 KiB of static arenas for a capability its own default (`user_zone_budget = 0`) never armed. 512 is not zero because a shipping title *is* an intended user of data-defined zones; it is the number at which the arenas cost 46 KiB instead of 234 KiB.

The two ranges have **separate counters**, so a game exhausting its budget cannot consume an engine id (G11) — and after B3 that statement is true of the traffic that will actually exist, not only of `register_zone` calls.

**Rejected.** defmt-style linker-section interning (the consecutive-address property is an ELF linker-script artifact; this box is windows-gnu/PE-COFF). A fixed `#[repr(u32)]` enum per subsystem (that *is* `VbTimedPass`, whose widening hazard we are removing).

## D19 — Two authoring paths, ONE registry, ONE store — partitioned by the DECLARING CRATE, not by the macro

| Path | Who | Cost | Tier-foldable | Partition |
|---|---|---|---|---|
| `declare_zone!` (static) | any crate — engine or downstream | ≤ 12 ns | yes | **its crate's** `profiling_partition!` |
| `register_zone` → `DynZoneHandle` (dynamic) | zones defined by data / config / script / mods | ≤ 14 ns (≤ 18 ns across an FFI/script boundary) | **no** — a data zone has no compile-time tier | always `User` |

`declare_zone!` is exported from `boyko_diag::profiling_abi` and re-exported through `boyko_ecs::prelude`, so **X1 needs no new mechanism**: a game plugin crate writes `declare_zone!` verbatim and pays the engine's price. What rev 4 adds is the one line at that crate's root — `profiling_partition!(User)` — without which it does not compile.

**Rev 3's partition was keyed on the wrong thing (B3).** It read "static ⇒ engine, dynamic ⇒ user", while recommending the *static* macro as the game path — so the recommended game path minted engine ids into the engine ring, and G11/G20 both passed because both exercised only `register_zone`. The key is now the declaring crate (D6), which is the authorship boundary the property is actually about. Two partitions, each with its own gate, and **each gate's RED is now produced by the recommended game path**:

1. **Id space** (D6): `ENGINE_ID_NEXT` and `USER_ID_NEXT` are separate counters over disjoint ranges. A `User`-partition crate exhausting `user_zone_budget` gets `W9210` and a refused mint — *whether it used `declare_zone!` or `register_zone`* — while the engine's next `declare_zone!` still mints. **G11**, whose game leg is now a static `declare_zone!` in a `profiling_partition!(User)` crate.
2. **Ring capacity**: `ZoneLane` is **two SPSC regions** — `ENGINE` and `USER`. The region is a **compile-time constant of the declaring crate**, so there is no runtime branch. A runaway game scope fills the `USER` region and drops `USER` samples; `engine_overflow` stays 0. **G20 — the extension's headline gate**, whose runaway leg is likewise a static site in a user-partition crate.

**A gate needs both partitions in one process, and one crate can only be one partition** — so G11/G20 are two-crate by construction: the engine zones come from `boyko_ecs` itself (`__frame`, `__main_run`), the user zones from the test target's own crate, which declares `profiling_partition!(User)`. That is the real topology, not a simulation of it, and rung 15's acceptance leg uses `boyko_demo` — a genuine game crate — for the same reason.

**Residual, named:** an out-of-workspace crate could still write `profiling_partition!(Engine)` — but only by *failing the const-assert*, since its `CARGO_PKG_NAME` is not in `ENGINE_PACKAGES`. Within the workspace, a member that lies is one greppable line and is pinned by a tidy test. There is no per-site escape at all, which was rev 3's actual hole.

Cost, stated: lane control blocks 256 B/lane (20 KiB `.bss` dev, 8 KiB shipping); per-region capacity **1024** samples in `dev`/`editor`, 128 in `shipping` (`REGION_CAPACITY`, a per-profile const — S9). 1024 samples at ~400 engine samples/frame is **2.5 frames of burst headroom**, against a fold that runs every frame. That is down from rev 3's 5 frames and it is the price of B1's 24 B record: holding 2048 would have put the dev lane slab at 7.5 MiB against a 7 MiB budget. `G4` makes any shortfall visible rather than silent, `BOYKO_PROFILE=custom` can raise it, and the USER region's overflow is isolated from the engine's by G20.

## D21 — `ZoneTier` and `GLOBAL_TIER`: the compile-time half of the emission gate

```rust
#[repr(u8)] pub enum ZoneTier { Always = 0, Dev = 1, Deep = 2 }
pub const GLOBAL_TIER: ZoneTier = /* from boyko_diag's build.rs, per BOYKO_PROFILE */;
```

The macro's first gate is `const { $handle::TIER as u8 <= GLOBAL_TIER as u8 } && ARM_MASK…` — **read from the `mod` companion `declare_zone!` emits, never through the handle static.** Reading it through the static is `E0080: constant accesses mutable global memory`, because the handle carries an `AtomicU16`; that form was specified here for four revisions and does not compile (measured, rustc 1.95.0). See §`declare_zone!` for the two-item expansion and the probe results. **A short-circuit `&&` over a `const false` deletes the arm and its operands**, which is the `log!` property D1 relies on and the only mechanism in this design that reaches literal zero per site.

| Tier | Contents | Shipping |
|---|---|---|
| `Always` | frame time, a small counter set, crash/telemetry-relevant gauges | **ships** |
| `Dev` | per-pass GPU zones, subsystem spans, histograms | folded out |
| `Deep` | per-system scheduler zones, round records, per-draw counters | folded out |

**The axis itself is not this plan's.** Rev 3 had a private axis (`BOYKO_PROFILING_TIER`, read by a new `crates/boyko_ecs/build.rs`) and the logging plan had another; rev 4 has one, and it is neither of theirs. `BOYKO_PROFILE` is read by exactly one build script in the workspace — `crates/boyko_diag/build.rs` — and emits `GLOBAL_TIER`, `LANE_COUNT`, `REGION_CAPACITY`, `ENGINE_ZONE_SLOTS`, `MAX_USER_BUDGET` and `DYN_NAME_BYTES` for this plan to re-export. The profile→const table, the `custom` rule, the `compile_error!` on a stray per-knob override, the three-independent-header-facts requirement and the CI leg arithmetic all live once, in `seam/build-axis`. **`crates/boyko_ecs/build.rs` is NOT created** (rev 3's integration row is withdrawn).

Orthogonally, `feature = "profiling-analysis"` gates the `compat` matrix, the `intervals` ring, `ConcurrencyReport`, the contrast machinery and the TOML writer — the parts a shipping title never runs.

**One cost belongs to the emission path and is stated here.** ~12 KiB of dead `.bss` remains for folded `ZoneHandle` statics. **The *second* cost this line used to claim does not exist**, and the reason is name resolution rather than that `.bss` fact: `zone!`'s expansion names the identifier in both the gate (`const { $h::TIER <= GLOBAL_TIER }`, read from the `mod` companion) and the body (`ZoneGuard::open(&$h)`), so a typo in a `Deep` zone name is a hard `E0425` in **every feature-on profile including shipping**, not invisible in one. The invisibility cost belongs to the FEATURE axis alone (G1(a)), where `#[cfg]` deletes the macro before name resolution. **G14 is re-specified in rev 4 (B5):** rev 3's version asked a per-binary object-symbol census to report the recorder symbol **absent** (clause 1) and **present** (clause 2) at once; a census answers "is symbol S referenced in this object", per binary, and cannot attribute a reference to a site, so the two clauses contradicted each other and no RED existed. It is replaced by a per-site mechanism plus a behavioural one (`profiling/05-LADDER-GATES.md`).

## D15 — Lane buffers are allocated once and NEVER freed; disarm is a mask store

Rev 1 freed the lane slab at disarm behind a quiescence argument that covered workers only. The stated guard was worse than absent: `is_in_system_run()` (`crates/boyko_threadpool/src/tls.rs:83`) reads **the calling thread's own TLS** and can never observe another thread. And the cited precedent does not transfer: `ThreadLaneWriter`'s `Sync` clause 2 (`crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:102-106`) rests on *"`update_events`, which takes `&mut EventDispatcher` — the `&mut` acts as the synchronisation point"*; a `static LANES` has no `&mut` to stand in for that clause.

**Decision: there is no free** — and, after B4, no *owner* either. The reservation is created, committed, published and `mem::forget`-ed at the first `arm()` (D8), so "it lives for the process" is structural rather than asserted: there is no value left whose `Drop` could unmap it. `arm`/`disarm` only store `ARM_MASK` and reset cursors. `buf` is published once, `Release`, and **never nulled** — which is what lets A1 step 9 store without a null test, and which is precisely why nulling it at `disarm` is *not* an option: an emitter that passed the mask gate before the clear could load a nulled `buf` after it.

**This is S12's policy applied, not a per-plan exception** (`substrate/never-freed-storage`). *Extent known at compile time ⇒ `.bss`; extent chosen at run time from config ⇒ `VmReservation`.* The lane control blocks, `REGISTRY` and the dynamic arenas are the first case; the sample slab and the columns are the second. The same rule governs the logging plan's tables, and one gate (`boyko_diag::section_report`) proves both — so a toolchain change reds one gate, not two that disagree about which is authoritative.

**One consequence of the panic-hook rule (S5, `seam/lifecycle-order`):** the telemetry double buffer and its file handle are **not** in the `Profiler` `Resource`. They live in a `boyko_app::profiling::stream` process-static — compile-time extent, therefore `.bss`, consistent with S12 — because `flush_on_panic` runs from the panic hook, takes no arguments and may not touch the `World`.

**Honest consequence:** "disarmed = a few KiB of BSS and nothing else" is true only *before the first arm*. After a first arm, disarmed resident cost is the full committed reservation. Stated in the artifact and in the budget table.

**S13 changes nothing here, and that is worth recording so a later edit does not "fix" a row that is already correct.** D8/D15 already put every allocation, commit and publication inside `arm()`, which runs outside any system with `debug_assert!(!is_in_system_run())`. **`arm` IS the enable path.** With the runtime flag off, `arm` never runs, no page is committed, no `buf` is published and the `.bss` control blocks are never touched — so their cost is reserved address space, not resident memory. What S13 *does* add is the standing prohibition: **nothing in `boyko_diag` may be touched, calibrated, spawned or committed at process start**, which for this plan means the clock calibration in D3 moved from boot to enable and nothing else did.

---

## Data structures — the emission half

```rust
// ══════════════ boyko_diag::profiling_abi ══════════════
// Zero dependencies. Allocates NOTHING. Contains no Resource and names no World.

#[repr(u8)] pub enum Channel  { SchedulerCpu=0, GpuPass=1, Counter=2, Frame=3,
                                User0=4, User1=5, User2=6, User3=7 }
#[repr(u8)] pub enum ZoneKind { Span, Counter, Gauge }
#[repr(u8)] pub enum GpuStage { TopOfPipe, BottomOfPipe, NotGpu }
#[repr(u8)] pub enum Unit     { Ticks, Count, Bytes, Ratio }
#[repr(u8)] pub enum ZoneTier { Always = 0, Dev = 1, Deep = 2 }
#[repr(u8)] pub enum Region   { Engine = 0, User = 1 }   // compile-time const at every site

/// Immutable, `&'static`, one per site. NEVER on the emission path.
#[repr(C)]
pub struct ZoneDesc {
    pub name: &'static str,   // REQUIRED by declare_zone! -> cannot be forgotten (the property
                              // VbTimedPass::label() bought with a hand-maintained table)
    pub file: &'static str, pub line: u32,
    pub channel: Channel, pub kind: ZoneKind, pub stage: GpuStage, pub unit: Unit,
    pub tier: ZoneTier, pub region: Region, pub scope_bit: u8,
    pub group: u16,           // PartitionGroup; 0 = none
    pub system_index: u16,    // != u16::MAX  =>  intervals retained for overlap analysis (D9/F19c)
}

#[repr(C)] pub struct ZoneHandle    { desc: &'static ZoneDesc, id: AtomicU16 }
#[repr(C)] pub struct DynZoneHandle { id: ZoneId, arm_bit: u64, /* 16 B, Copy, Send+Sync */ }

/// THE record. 24 B, 2.67 per cache line, one shape for every kind (B1).
///
/// `stamp` is ABSOLUTE TSC and is present for EVERY kind: a frame-relative u32 would
/// need a shared per-frame base (a coherence miss on every worker at frame start) and
/// would overflow on a >1.4 s frame — the hitch most worth recording. Absolute u64 is
/// also what makes frame attribution a merge (A2) and the overlap matrix epoch-free.
///
/// The payload has its OWN 64 bits. Rev 3 overloaded `begin` with three meanings
/// (TSC / value / dur-high-bits) and the fold read it before dispatching on kind, so
/// counters, gauges and long spans were all attributed by a field that was not a time.
#[repr(C)]
pub struct Sample {
    stamp: u64,   // Span: TSC at OPEN. Counter/Gauge: TSC at the emit call. THE attribution key.
    value: u64,   // Span: duration in TSC ticks (u64 => no saturation, no Extension record).
                  // Counter: the increment (summed within a frame). Gauge: the level.
    zone:  u16,
    flags: u16,   // [0..1] kind (Span|Counter|Gauge) | [2] gpu-origin | [3..15] reserved
                  // (no `saturated` bit — nothing saturates; no depth field — D3a)
    _pad:  u32,   // named, so the layout is pinned rather than incidental
}
const _: () = assert!(size_of::<Sample>() == 24 && align_of::<Sample>() == 8);

/// Writer and reader halves on SEPARATE lines. All mutable state is atomic:
/// no UnsafeCell, no plain field mutated through `&'static` (rev 1 had both).
#[repr(C, align(64))]
struct RegionWriter {
    buf:      AtomicPtr<Sample>,  // published ONCE at first arm (Release), never nulled (D15)
    write:    AtomicU32,          // Relaxed read by the sole owner; Release store after the bytes
    overflow: AtomicU64,          // dropped samples; MONOTONE, never cleared (D24a / Q2(b))
    _pad:     [u8; 40],
}
#[repr(C, align(64))] struct RegionReader { read: AtomicU32, _pad: [u8; 60] }
#[repr(C, align(64))] struct RegionLane   { w: RegionWriter, r: RegionReader }

/// FOUR distinct lines: engine writer / engine reader / user writer / user reader.
/// The engine/user split is a false-sharing fix as well as an isolation fix — a game's
/// `write` cursor never invalidates the engine's (D19).
#[repr(C, align(64))] struct ZoneLane { engine: RegionLane, user: RegionLane }
const _: () = assert!(size_of::<ZoneLane>() == 256);

// ── lane identity: OWNED BY boyko_diag::lane, re-exported here for reference only (S3) ──
pub use boyko_diag::lane::{
    LANE_WORKER_MAX,   // 64 == boyko_threadpool::MAX_WORKERS (thread_pool.rs:49)
    LANE_DISPATCHER,   // 64
    LANE_HOST,         // 65
    LANE_COUNT,        // 80 in EVERY profile — no profile axis (Q1)
    LANE_UNCLAIMED,    // u16::MAX
    lane, set_lane, claim_lane, release_lane,
};
// thread_local! { static LANE: Cell<u16> } lives in boyko_diag and has NO Drop guard (S3).

pub const REGION_CAPACITY: u32 = /* 1024 dev/editor, 128 shipping — per profile (D19/S9) */;
// dev:      80 lanes x 2 regions x 1024 x 24 B = 3.75 MiB of sample slab
// shipping: 80 lanes x 2 regions x  128 x 24 B =  480 KiB   (was 192 KiB at 32 lanes — Q1)

static ARM_MASK: CachePadded<AtomicU64>;             // 0 == disarmed. Own line, read-mostly (D20)
static LANES:    [ZoneLane; LANE_COUNT as usize];    // .bss: 20 KiB in EVERY profile (Q1)
static REGISTRY: [AtomicPtr<ZoneDesc>; ZONE_ID_SPACE];   // .bss: 56 KiB dev / 6 KiB shipping
static DYN_DESCS: SyncCells<ZoneDesc, MAX_USER_BUDGET>;  // .bss: 144 KiB dev / 24 KiB shipping (A7)
static DYN_NAMES: SyncCells<u8, DYN_NAME_BYTES>;         // .bss:  64 KiB dev / 16 KiB shipping
// SyncCells<T, N> is boyko_diag::storage's ONE shared never-freed shape (S12), used by both
// subsystems and proved by ONE gate (`boyko_diag::storage::section_report` — G22a/G22b).
// Every extent above is a compile-time const, which is exactly why they are .bss and not VM.
```

**Every `.bss` figure above is a RESERVED extent, not a resident cost** (S13). `.bss` is demand-zero: with the runtime flag off nothing writes `LANES`, `REGISTRY`, `DYN_DESCS` or `DYN_NAMES`, so the image carries no raw data for them and the process touches no page. The limit of that claim is stated once, by `substrate/section-report`: the gate proves **absence of raw data in the image**; that the OS leaves an untouched page uncommitted is UNPROVEN and is not asserted.

---

# Part 2 — The durable store (`boyko_ecs::ecs::core::profiling`)

## D8 — Storage: a `Resource`-owned FRAME-MAJOR SoA store on `VmReservation`; the stride is fixed at arm

**Layout fork, decided with numbers — recomputed (F15), and recomputed AGAIN in rev 4 for `count: u32` (M9).** Rev 2's table said "≤ 256 lines ≈ 16 KiB" and omitted the `label` column entirely. Correct arithmetic, per frame row, at stride `Z`:

| Column | Width | Bytes at `Z = 1024` | Lines |
|---|---|---|---|
| `total` | `u64` | 8192 | 128 |
| `count` | **`u32`** (was `u16` — M9) | 4096 | 64 |
| `min` | `u32` | 4096 | 64 |
| `max` | `u32` | 4096 | 64 |
| `label` | `u8` | 1024 | 16 |
| **row total** | **21 B/zone** | **21 504 B = 21 KiB** | **336** |

**Why `count` widened.** One fold consumes at most `LANE_COUNT × 2 regions × REGION_CAPACITY` = 80 × 2 × 1024 = **163 840** samples, and every one of them may target a single zone (a per-entity dynamic zone, a per-draw counter — precisely the "as much data as possible" case). `u16` wraps at 65 535 **silently**, after which `total`/`min`/`max` describe a different sample set than `count` does, no drop class covers it and no gate exercises it. `u32` cannot wrap by the same arithmetic (163 840 ≪ 2³², and a cell is zeroed when its frame row is recycled) — a proof, not a bound, so no saturation counter is needed. Cost: +2 B/zone/frame, +61 KiB retail, +230 KiB dev.

| Layout | Fold (per frame, hot) | Window reduction (cold, once) |
|---|---|---|
| zone-major `[zone*W + f]` | ~400 live zones × 5 columns = **2000 distinct lines ≈ 125 KiB**, far over L1d | sequential per zone |
| **frame-major `[f*Z + zone]`** | **336 lines = 21 KiB** at `Z = 1024` | constant-stride gather (stride `21·Z` split per column), `WINDOW` reads per zone per column, hardware stride prefetcher applies |

**Frame-major wins by ~6× on the frequent side**, and the strided side runs `#[cold]` once per window. **Decided — but the L1d claim is qualified honestly:** 21 KiB of columns plus the fold's ~9.6 KiB of sequential lane reads (400 samples × 24 B, B1's wider record) is **30.6 KiB against a 32 KiB L1d**. It fits; it is *tight* — tighter than rev 3's 25.4 KiB, and this is where the record widening is actually paid for. At `Z = 2048` it does not fit, which is exactly what `W9211` reports, and `fold_cost`'s `zone_stride` legs **measure the cliff rather than assuming it**.

**Arm-time `zone_stride` (X5).** `Z = ENGINE_ZONE_SLOTS + armed_user_budget`, fixed at `arm` and const for the session. **The `dev` row of the table above says `Z = 1024` and its own `.bss` breakdown says `ENGINE_ZONE_SLOTS = 4096`; those cannot both hold, and the contradiction is recorded rather than resolved here.** Two of the three sizing rows are self-consistent at `ENGINE_ZONE_SLOTS = 4096` (`REGISTRY` is `(4096 + 3072) x 8 = 56 KiB`, and the `user_zone_budget = 3072` row's `Z = 7168` is `4096 + 3072` exactly); only the `dev` default row's column figure requires 1024. Rung 2 implements the **formula**, which is stated as the rule and which the other two rows agree with, and reports the consequence through `W9211` instead of picking the constant: `ENGINE_ZONE_SLOTS` is a per-profile const that the single build axis owns at J1, and rung 2 is its first reader, not its author. `arm` twice with a different geometry ⇒ `E9213`. Above the L1d threshold `arm` still succeeds and emits `W9211` naming the measured working set — a game may legitimately want 2000 zones and pay for them, but it will be told.

**`WINDOW = 121`, not 120.** An even window makes every median the mean of the two middle samples — a value no frame produced, sitting half a lattice tick off. That is precisely how the 16 ns lattice was first mis-derived (the measured fact is `profiling/statistics-discipline`'s). 121 frames ≈ 2.02 s at 60 Hz. Column bytes: `21 × 1024 × 121 = 2 601 984 B = 2.48 MiB` at the dev default. (Rev 3 quoted "2.35 MiB" for `19 × 1024 × 121`; that product is 2 353 664 B = **2.25 MiB** — 2.35 was the count in *millions of bytes* read as MiB. Corrected.)

**Backing store: the reservation has NO owner, by construction (B4).** Rev 3 kept the `VmReservation` in the `Profiler` `Resource` and argued that "the use-after-free class is deleted by construction" — but `impl Drop for VmReservation` at `crates/boyko_ecs/src/ecs/memory/vm.rs:263` unmaps, worker threads hold published `buf` pointers that are never nulled, and a world dropped in a multi-world test or at teardown would therefore dangle every one of them. That is the rev-1 UAF class re-entering through the *owner* instead of through `disarm`, and an argument cannot fix it — only a location can.

So the reservation is **created, committed, published and then deliberately forgotten**:

```
first arm:  vm = VmReservation::reserve(total_bytes)   // vm.rs:109
            vm.commit(0, total_bytes)                  // vm.rs:199
            VM_BASE.store(vm.base().as_ptr(), Release) // vm.rs:184; static AtomicPtr<u8>, .bss
            VM_LEN.store(vm.os_len(), Release)         // vm.rs:190 — NOT `reserved_bytes()`,
                                                       //   which does not exist in the tree
            mem::forget(vm)   // SAFETY/rationale: the lane `buf` pointers derived from this
                              //   base are published to every thread and are never nulled, so
                              //   unmapping is UB for the life of the process. Leaving no owner
                              //   makes "never freed" structural instead of asserted. The address
                              //   space is leaked on purpose; this is the one deliberate leak.
```

The `Profiler` `Resource` then holds a `base: NonNull<u8>` copied from `VM_BASE` plus **byte offsets** — never `&'static mut [T]` slices — and hands columns out through accessors that reconstitute a slice for the duration of the call. Rev 3's eleven `&'static mut` fields aliasing memory the same struct owned are two mutable paths to the same bytes; Tree Borrows flags exactly that, and the kernel's own precedent already avoids it (`VmColumn` keeps `base: NonNull<T>` + accessors, `crates/boyko_ecs/src/ecs/memory/vm_column.rs:88`). **No `Box<[T]>`, no `Vec`, no `&'static mut`** (F7 + B4). `Profiler::reserved_bytes()` returns `VM_LEN`, which is what G23a/G23b measures.

**`Send`/`Sync`, stated rather than assumed (B4).** A `NonNull<u8>` field makes `Profiler` `!Send`/`!Sync` while `Resource: 'static + Send + Sync + Sized` (`crates/boyko_ecs/src/ecs/core/resources/resource.rs:42`), so the type carries an explicit `unsafe impl Send for Profiler {}` / `unsafe impl Sync for Profiler {}` with three clauses: (a) every mutation happens outside the schedule, on the dispatcher/host thread (D16/A3), so there is never a concurrent `&mut`; (b) in-frame access is `Res<Profiler>`, shared-only, and the kernel's own resource borrow rules enforce it; (c) the base is write-once and the region is never resized, never moved and never freed (above), so no pointer derived from it can dangle. **That impl is in the unsafe inventory and on the Miri list — rev 3 had it in neither**, and `VmReservation`'s own doc (`vm.rs:82-84`) demands exactly this: *"owners that are shared across threads opt in with their own `unsafe impl` and their own exclusivity argument"*.

**Transport control blocks.** `static LANES: [ZoneLane; LANE_COUNT]` in `.bss` — 256 B per lane × 80 = **20 KiB in every profile** (four distinct lines per lane after the two-region split, D19). It read 8 KiB in `shipping` until Q1 deleted the profile axis. Each region's `buf: AtomicPtr<Sample>` is published `Release` once at first arm.

**Multi-world.** The rings are process-global; worlds are not. **v1 binds the profiler to exactly one world**: `ProfilerPlugin::build` records the `WorldId` in a global; a second registration is `boyko-E9204`. Enforced at bind time, not assumed.

## D16 — The instrument is outside its own primary number, and the number is defined across `Fixed×N` + `Main`

Rev 2 put the fold in `App::update` (`crates/boyko_ecs/src/ecs/core/app/app.rs:736`). **The windowed host does not call it**: `crates/boyko_app/src/runner.rs:1321` calls `app.update_with_delta(dt)` directly, and `App::update` (`:736-744`) merely computes a delta and forwards. In the only configuration that has a GPU channel, **the fold would never have run** — lanes fill, `overflow` climbs, no frame ever seals — and nothing in rev 2's gate list caught it, because the unit tests drive `App::update` (F2).

**The fold moves to the top of `App::update_with_delta` (`app.rs:655`), the single funnel both entry points share**, before step ① (`Time::advance_with`). `App::update` needs no change.

Second half of F2: rev 2's "primary CPU number" was *"the `Schedule::run` span"*, but `crates/boyko_app/src/runner.rs:943` documents the frame as *"Time → events → Fixed×N → Main"* — **two schedules, and `Fixed` runs N times.** "The `Schedule::run` span" is not one interval, and "the fold is outside the primary number" was undefined across N+1 runs.

**Definition, stated once:**

| Zone | Bracket | Cardinality |
|---|---|---|
| `__frame` | `update_with_delta` entry (**after** the fold returns) → exit | 1 per frame — **this is the primary CPU number** |
| `__events` | step ③ `update_events` | 0 or 1 |
| `__fixed_step` | one `fixed.run(world)` inside step ④ | **N** per frame; `FrameRecord.fixed_steps` records N |
| `__main_run` | step ⑤ `schedule.run(world)` | 1 |
| `__fold`, `__reduce`, `__hist_fold`, `__telemetry_write` | the instrument's own work | outside `__frame` by construction |

`FrameRecord` carries `run_gross` (= `__frame`), `fixed_total` (Σ over the N substeps), `main_total`, `instrument_measured` and `instrument_estimated`. All are artifact fields (S1: nothing here has a console form).

**`instrument` is split (F18).** Rev 2 defined `instrument = Σ __fold + __reduce + __cpu_null + zone_count × measured_zone_cost` and then printed `run_net = run_gross − instrument`. The last term is an **estimate from a different binary and profile**, injected into a per-frame number — in the document that refuses to print unresolvable deltas and that cites `median(off)+median(dur) ≠ median(off+dur)`. So:

- `instrument_measured` = Σ of the instrument's **own zones**, measured in-band this frame.
- `instrument_estimated` = `zone_count × zone_cost_ticks`, carrying `zone_cost_provenance` (bench id + `build_hash`).
- **`run_net = run_gross − instrument_measured_inside_frame`. The estimate is never subtracted from anything.** It is recorded beside, labelled, with its provenance.

## D9 — Concurrency = STATIC compatibility vs OBSERVED interval overlap, at the kernel's own system bound

Rev 1 could not compute its own headline. Rev 2 could, but at `MAX_SYSTEMS = 512` against a kernel cap of **1024** (`schedule_builder.rs:70`), with no counter for what it truncated — M8's exact defect at 2× the bound (F5). And its `intervals` write was an **assignment** to one slot per `(frame, system)`, while the host frame is *"Time → events → Fixed×N → Main"* (`runner.rs:943`), so a system in `Fixed` overwrote itself N−1 times per frame (F19b). And `sys` was not derivable — `Sample` carries `zone: u16` only (F19c).

All three fixed:

- **Declared** = the static compatibility matrix, snapshotted from `ConflictGraph` at arm, at `MAX_SYSTEMS = MAX_SYSTEMS_PER_SCHEDULE = 1024`: `compat`, 1024×1024 bits = **128 KiB**. Pair `(i,j)` is compatible iff no access conflict and no ordering edge in either direction. The snapshot covers the **one schedule named in `ProfilerConfig::analysed_schedule`**; systems in other schedules are counted in `systems_unanalysed`, never silently dropped.
- **Observed** = an **append ring**, not an assignment: `intervals: [Interval; OVERLAP_FRAMES × INTERVALS_PER_FRAME]` with `OVERLAP_FRAMES = 8`, `INTERVALS_PER_FRAME = 2048`, `Interval { begin: u64, dur: u32, sys: u16, occ: u16 }` = 16 B → **256 KiB**. A system running N times per frame appends N intervals; overflow increments `intervals_dropped`.
- **`sys` resolution**: `ZoneDesc.system_index: u16` is set at mint time in `try_build` (the builder knows the index), and `arm` builds a `sys_of: [u16; zone_stride]` side table — 2 KiB, L1-resident, one indexed load per system-tagged sample at fold. The same shape as `hist_of` (D22).
- `RoundRecord { frame: u32, round: u16, dispatched: u16, begin: u64, end: u64 }` = 24 B keeps dispatch *shape* only (rounds per frame, wave width, round span). No membership mask, hence **no truncation and no silent wrong answer**. `MAX_ROUNDS_PER_FRAME = 32`; overflow is counted and reported.

`ConcurrencyReport` prints, per compatible pair that both ran: `declared=1 observed_frac=x.xx`, plus the aggregate **serialisation index** = 1 − (Σ observed overlap / Σ declared-compatible-and-both-ran).

**All of D9 is behind `feature = "profiling-analysis"` (default ON in dev, OFF at retail).** `compat` + `intervals` = 384 KiB, which a shipping title has no use for.

### What D9 SHIPPED at rung 3d, against the four paragraphs above

The four bullets are kept verbatim because their *arguments* are what the implementation followed; what it stored differs in four places, each argued in `profiling/05-LADDER-GATES.md` §"What rung 3d SHIPPED" and each raised in `docs/OPEN-QUESTIONS.md`.

| Specified | Shipped | Why |
|---|---|---|
| `compat`, 128 KiB snapshotted at arm | **not built** — the report reads the live `ConflictGraph` | a schedule is built once at `App::finish` and never rebuilt, so the live graph *is* the snapshot. Residual named: a later rung that makes schedules rebuildable must snapshot or refuse |
| `sys_of`, 2 KiB built at arm; `Interval.sys` | **not built**; the field is `Interval.zone` | rung 3a put system → zone on `SystemMeta.zone`, which the schedule owns, so zone → system resolves at **report** time. A field named `sys` holding a zone is the name this corpus exists to catch |
| `RoundRecord`, 90.8 KiB, `MAX_ROUNDS_PER_FRAME = 32` | **not built** — two zone sites, `__round` (Span) and `__round_width` (Counter) | rounds/frame is `__round`'s `count`, round span its `total`/`min`/`max`, wave width `__round_width`'s. No storage, no truncation, no `rounds` drop class — and no second write path into the reservation from a dispatcher that does not hold `&mut EcsMaster`. **Lost:** the width↔span correlation within one round |
| `ConcurrencyReport` prints per compatible pair | aggregate in one pass + a per-pair call | 1024 systems is 523 776 rows to answer an aggregate question. `pair_overlap(a, b)` gives the corpus's `declared=1 observed_frac=x.xx` for the pair a caller names |

`intervals` shipped as specified — an **append** ring, `OVERLAP_FRAMES = 8`, `INTERVALS_PER_FRAME = 2048`, 16 B per record, 256 KiB — and so did `intervals_dropped`, with one distinction the corpus does not state and that the implementation had to make: **a full bank is a loss and is counted; a frame outside the eight-frame horizon is a stated bound and is not.** The measurement of an out-of-horizon span is in its column cell either way, so counting it would report one sample under two headings — the double-count `substrate/loss-fold` exists to remove. Both directions have a run RED.

Two figures the report carries that the corpus does not name: `conflicting_overlapped` (pairs the graph declared incompatible whose intervals nevertheless intersected) is **reported and never asserted on** — two spans measured on two cores are two `rdtsc` readings, and cross-core skew can make an abutting pair read as overlapping by a handful of ticks. And `serialisation_index()` returns `Option<f32>`: `None` when no compatible pair co-ran, because `1.0` would report perfect serialisation where the honest answer is that nothing ran.

## D7 — The stage table is a per-zone declaration, and partition sums are CHECKED — per frame, never over medians

`ZoneDesc.stage: GpuStage` and `ZoneDesc.group: PartitionGroup`. The window reducer refuses to sum a group unless **every** member declares `BottomOfPipe` and their intervals are non-overlapping and contained in the group's run bracket; otherwise it emits the members individually and writes `sum = NOT_VALID (mixed stage)` — an **artifact field**, not a printed line (S1/S7: the reducer has no console form).

**Why.** `begin_stage`'s argument (`crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:333-365`) is correct and currently enforced by nobody: consecutive `BOTTOM` stamps are prefix-completion times, prefixes nest, so their intervals exactly partition the span; a `TOP` stamp recorded *after* a `BOTTOM` stamp may legally report an earlier time. Today `froxel_total_ns` sums three independent brackets and discloses it only in a prose `NOTE:`.

**The sum is formed per frame, then reduced** — `median_f(Σ_members)`, never `Σ_members(median_f)`. `median(a) + median(b) ≠ median(a + b)`, and in VG R3 P4-6 that inequality was crossed by 144-240 ns on a real reading. The window reducer has **no** API that adds two reduced statistics; the addition happens in the frame-major row, which is the layout that makes it a single sequential pass. This is the storage-side reason the layout fork was decided the way it was.

**Trade-off.** VB-P1d slots 0/1/2 stay `TopOfPipe` and can therefore never join a partition group. Correct — they never could.

## D24 — Drop accounting stays honest at session scale

**The vocabulary is `boyko_diag::loss`, shared with the logger (S8) — `substrate/loss-vocabulary` owns it, not this plan.** `LossClass { Overflow, Unclaimed, Late, Refused, Device, Sink, Rotation, Budget }`, `LossCell` (64 B-aligned, lane-owned), `LossTotal`, `LossStatus { Measured, Unproven, UnprovenLossy, UnprovenSampled, UnprovenUnsunk }`. Accumulation is **`u64`, never saturating**: on x86-64 a `lock xadd` costs the same at 4 and 8 bytes, and the lane-owned cell needs no RMW at all, so the logging plan's saturating `u32` (and its `SATURATED(≥4294967295)` census token, which no reader could compare) does not survive. A game reads **one** resource, `DiagCensus { log: LogCensus, prof: ProfCensus, lossy: bool }`.

Sharing removes a second-order defect *by construction* rather than mitigating it: with the counters in the leaf, the **report** of a profiler drop is a read of a counter, not a log record that can itself be dropped. Rev 3's design reported profiler drops *through* the logger, so under load — precisely when drops occur — the report of the loss would be dropped and counted as a *logger* loss, double-counting one event with no rule saying which counter was authoritative.

- **(a) THERE IS NO CLEAR. RESOLVED — `substrate/loss-fold`'s Q2 answered (b), and this row is rewritten to what shipped.** Every counter is monotone and is never cleared by anybody; each consumer owns a `LossSeen` per `(row, class)` — and, for the regions, a `[[u64; 2]; LANE_COUNT]` in the `Profiler` — and folds `cur.wrapping_sub(last)` (`boyko_diag::loss::delta_since`, `boyko_diag::sample::overflow_since`). Exactness then follows from the **shape of the datum**, a counter that only ever goes up, with no discipline left for a caller to forget. **One gate still serves both plans** (G4b = logging's G11 — S8); what changed is its mechanism and therefore its RED, not its claim.

  *What this row said for four revisions, kept because the reasoning is the argument for (b):* the clear was to be `fetch_sub(observed)` rather than `store(0)`, so that a producer increment between the fold's load and its clear survived. That closes the **consumer** side only. With the owner's increment a plain `load; add; store` — which is what "no lock prefix" buys — a consumer `fetch_sub` landing between that load and that store is overwritten, while the value it subtracted has already been folded into the consumer's total, so the one loss event is counted **twice**: the exact double-count S8 exists to remove. The monotone form has no such window because the consumer never writes the cell at all. `boyko_diag` shipped `loss.rs` with no `fold_into`, no `store(0)` and no `fetch_sub`, and their absence is the whole correctness argument rather than a simplification.

  **Consequence for `G4b`'s showable RED, stated here so the two do not drift again:** the RED is no longer *"replace `fetch_sub(observed)` with `store(0)`"* — there is nothing to replace. It is **"replace `overflow_since(lane, region, seen)` with `overflow(lane, region)`"** in the fold, i.e. fold the monotone total instead of the consumer-side delta ⇒ every fold re-adds every earlier refusal ⇒ the counter runs away. MEASURED at rung 2: the injected figure of 5 read 10 on the second fold.
- **(b) Every drop class is attributed, and every ring class is attributed PER REGION.** The 18 classes and their `LossClass` mapping: `engine_overflow`, `user_overflow` (`Overflow`) · `unclaimed` (`Unclaimed`) · `late` (`Late`) · `zones_refused`, `user_registrations_refused`, `telemetry_zones_refused` (`Refused`) · `gpu_lost`, `gpu_slots_abandoned`, `gpu_frame_deadline`, `gpu_budget` (`Device`) · `telemetry_write_errors` (`Sink`) · `rounds`, `intervals_dropped`, `hist_saturations`, `span_over_range`, `clock_epoch_breaks`, `systems_unanalysed` (`Budget`). All `u64`. All **release-live** — a reporting obligation that vanishes in release is the vacuous-gate pattern by another route. (`span_over_range` is new in rev 4: a span whose duration exceeds `u32::MAX` ticks is exact in `total`/`count` but clamps the `min`/`max` columns, so its `(frame, zone)` cell is labelled `OVER_RANGE` and `resolve` refuses the leg through the existing `LabelNotMeasured` path. `count_saturations` does **not** exist, because `count` is now `u32` and provably cannot wrap — M9.)
- **(c) Non-wrap proof.** A `u32` region counter can gain at most one increment per refused sample per fold interval; one frame at 60 Hz cannot produce 2³² refusals from 80 lanes at 1024 slots each. Accumulated into `u64`, which at 10⁶ drops/s wraps in 585 000 years.
- **(d) `resolve` refuses a leg with any drop** (`WindowIncomplete`). **This tightens the engine side:** a bench that drops now produces no number instead of a wrong one (X8).

---

## Data structures — the store half

```rust
// ══════════════ boyko_ecs::ecs::core::profiling ══════════════

pub const WINDOW: usize = 121;                  // ODD, deliberately (S4). ~2.02 s at 60 Hz
const _: () = assert!(WINDOW % 2 == 1);
pub const MAX_SYSTEMS: usize = 1024;            // == schedule_builder::MAX_SYSTEMS_PER_SCHEDULE
pub const OVERLAP_FRAMES: usize = 8;
pub const INTERVALS_PER_FRAME: usize = 2048;
pub const MAX_ROUNDS_PER_FRAME: usize = 32;
pub const MAX_LEGS: usize = 8;
pub const CONTRAST_ZONES: usize = 16;

/// Dispatch SHAPE only — no membership mask, hence no truncation (D9).
#[repr(C)] pub struct RoundRecord { frame: u32, round: u16, dispatched: u16, begin: u64, end: u64 }

/// Retained per-system interval, APPENDED (never assigned) so a Fixed system running
/// N times per frame contributes N intervals (F19b).
#[repr(C)] pub struct Interval { begin: u64, dur: u32, sys: u16, occ: u16 }
const _: () = assert!(size_of::<Interval>() == 16);

#[repr(u8)] pub enum FrameState { Pending, Sealed, Partial }

/// 88 B, align 8 — computed field by field (F22 recomputed rev 2's wrong 72).
#[repr(C)]
pub struct FrameRecord {
    frame: u32, state: FrameState, flags: u8, rounds: u16,          //  8
    fixed_steps: u16, clock_epoch: u16, drops: u32,                 //  8   (D16: N is recorded)
    cpu_begin: u64, cpu_end: u64,                                   // 16
    run_gross: u64,                                                 //  8   __frame
    fixed_total: u64, main_total: u64,                              // 16
    instrument_measured: u64, instrument_estimated: u64,            // 16   split (F18)
    gpu_total: u64,                                                 //  8
    wall_ns: u64,                                                   //  8   labelled with its bound
}
const _: () = assert!(size_of::<FrameRecord>() == 88);   // 8+8+16+8+16+16+8+8 = 88

/// FRAME-MAJOR columns: index [frame * zone_stride + zone]  (D8, decided with numbers).
/// Every column is a BYTE OFFSET into the process-lifetime reservation reached through
/// `base` — never a `&'static mut` slice aliasing memory this struct owns (B4).
/// Accessors reconstitute a slice per call; the sizes in the comments are the extents.
pub struct Profiler {
    base: NonNull<u8>,                 // copied from VM_BASE at arm; write-once (D8)
    zone_stride: u32,                  // ENGINE_ZONE_SLOTS + armed_user_budget, fixed at arm

    off_total: u32,   // [Z*121] u64  1024*121*8 = 991 232 B = 968 KiB. Span: Σ ticks; C: Σ incr
    off_count: u32,   // [Z*121] u32  1024*121*4 = 495 616 B = 484 KiB   (u16 -> u32: M9)
    off_min:   u32,   // [Z*121] u32  1024*121*4 = 495 616 B = 484 KiB
    off_max:   u32,   // [Z*121] u32  1024*121*4 = 495 616 B = 484 KiB
    off_label: u32,   // [Z*121] u8   1024*121*1 = 123 904 B = 121 KiB
                      //   MEASURED / NOT_BRACKETED / TORN / LOST / OVER_RANGE (D24b)
                      // columns total = 21 B * Z * 121 = 2 541 KiB = 2.48 MiB

    off_lifetime: u32,  // [Z] retention tier B: 24 B each = 24 KiB          (D22)
    off_hist_of:  u32,  // [Z] zone -> hist slot, 0 = none                   (D22)
    off_hists:    u32,  // [cfg.hist_slots] 400 B each
    off_sys_of:   u32,  // [Z] zone -> system index, L1-resident             (F19c)

    off_frames:   u32,  // [121] FrameRecord   121 * 88 = 10 648 B = 10.4 KiB
    off_rounds:   u32,  // [121*32] RoundRecord              = 90.8 KiB  (analysis/Deep)
    off_legs:     u32,  // [8*16] LegSummary                 =  6.0 KiB  (analysis)
    off_frame_begin_tsc: u32,  // [121] u64    121 * 8       =  0.95 KiB — the A2 cut

    #[cfg(feature = "profiling-analysis")]
    off_compat:    u32,        // 1024^2 bits                           128 KiB
    #[cfg(feature = "profiling-analysis")]
    off_intervals: u32,        // [8 * 2048] Interval                   256 KiB

    scope_entity: [Entity; 64],        // the ECS source of truth for ARM_MASK (D20)
    scope_count:  u8,
    clock:   ClockCalibration,         // read from boyko_diag::clock: ticks_per_ns, calib_cv,
                                       //   calib_rejected, clock_epoch (D3/S4)
    quantum: [u64; 8],                 // per channel; GPU from measured_quantum_ns (D11a)
    cursor:  u32,
    drops:   DropCounters,             // 18 u64 classes over boyko_diag::LossClass (D24b/S8)
}
// SAFETY obligation, stated: `base: NonNull<u8>` makes this !Send/!Sync while
// `Resource: Send + Sync` (resources/resource.rs:42), so `unsafe impl Send/Sync for Profiler`
// carries D8's three clauses and is on the Miri list. `TelemetryWriter` is NOT a field —
// the double buffer and file handle are a boyko_app::profiling::stream .bss static (S5/D23).
```

**Sizing, computed field by field, `WINDOW = 121`. Rev 3's rows omitted the `.bss` statics entirely (M10); this table carries them, and the retail row survives the correction.**

| Configuration | `.bss` statics | Sample slab | Columns | B/C | Analysis | Frames+rounds+legs+cut | GPU host | **Total** |
|---|---|---|---|---|---|---|---|---|
| **`shipping`** (`Always`, analysis off, `Z = 256`, `hist_slots = 0`, **80 lanes**, `REGION_CAPACITY = 128`) | **66 KiB** | 480 KiB | 636 KiB | 6.8 KiB | — | 11.4 KiB (no rounds, no legs) | 8 KiB | **≈ 1 208 KiB = 1.18 MiB** ⚠️ |
| **`dev`, armed, analysis off** (`Z = 1024`, 64 hist slots, 80 lanes, `REGION_CAPACITY = 1024`) | **284 KiB** | 3.75 MiB | 2.48 MiB | 52 KiB | — | 108 KiB | 8 KiB | **≈ 6.67 MiB** |
| **`dev`, armed, analysis on** | 284 KiB | 3.75 MiB | 2.48 MiB | 52 KiB | 384 KiB | 108 KiB | 8 KiB | **≈ 7.05 MiB** |
| **`dev`, `user_zone_budget = 3072` (`Z = 7168`)** | 284 KiB | 3.75 MiB | 17.4 MiB | 214 KiB | 384 KiB | 108 KiB | 8 KiB | **≈ 22.1 MiB**, and `W9211` fires |

`.bss` breakdown — the rows M10 found missing: **shipping** = `LANES` **20 KiB** + `REGISTRY` (256+512)×8 = 6 KiB + `DYN_DESCS` 512×48 = 24 KiB + `DYN_NAMES` 16 KiB = **66 KiB** (it read `LANES` 8 KiB / total 54 KiB at 32 lanes). **dev** = `LANES` 20 KiB + `REGISTRY` (4096+3072)×8 = 56 KiB + `DYN_DESCS` 3072×48 = 144 KiB + `DYN_NAMES` 64 KiB = **284 KiB**. Rev 3 carried the dev figures into *both* configurations (234 KiB of them uncounted in the retail row), which alone would have broken the ≤ 1 MiB claim at 873 + 234 = 1107 KiB. The fix is not to stop counting them: `MAX_USER_BUDGET`, `DYN_NAME_BYTES` and `ENGINE_ZONE_SLOTS` are **per-profile consts** (D6/D21), which is what held the shipping row at 908 KiB *with* the statics counted. **`LANE_COUNT` is no longer one of them** — Q1 deleted its profile axis, and the row is now 1 208.2 KiB. `MAX_USER_BUDGET`, `DYN_NAME_BYTES` and `ENGINE_ZONE_SLOTS` remain per-profile; that part of the sentence still holds and is why the `.bss` column is 66 KiB rather than 284.

**`shipping` sits at 1 208.2 KiB against a budget of 1 280 KiB — raised from 1 024 by an owner decision on 2026-08-08.** The row reached 908 KiB through five profile consts together — `REGION_CAPACITY = 128`, **`LANE_COUNT = 32`**, `ENGINE_ZONE_SLOTS = 256`, `MAX_USER_BUDGET = 512`, `hist_slots = 0` — plus the `profiling-analysis` `#[cfg]` removing `compat`, `intervals`, `rounds` and `legs` entirely. **Q1 removed the second of those five**: `LANE_COUNT` is 80 in every profile, so `LANES` grows 8 → 20 KiB and the sample slab 192 → 480 KiB, and the row lands at **1 208.2 KiB**. That was 184.2 KiB over the old budget for one revision; the budget is now **1 280 KiB** and the row has 71.8 KiB of headroom.

**Two things follow, and neither is a rounding argument. Both are why the answer was a bigger number rather than a smaller design.**

1. **The overrun is 288 of the 300 KiB in the sample slab, and the slab is the one COMMITTED term.** `.bss` is reserved extent whose resident cost is `touched_lanes × row`; the slab is committed at first arm for all `LANE_COUNT` lanes at once (D15). So the honest split of the +300 KiB is **+288 KiB committed, +12 KiB reserved**, and only the first is memory a player's machine actually holds.
2. **The repair that keeps both Q1 and the budget is to commit sample regions per lane on first use instead of for all 80 at arm.** A shipped title on an 8-core box claims roughly `workers + dispatcher + host ≈ 10` lanes, i.e. 10 × 2 × 128 × 24 B = **60 KiB** of slab against the 480 reserved — comfortably back under budget with the same constant. That edits **D15**, whose current wording ("committed once at first arm, never freed") is what forbids it, and D15 is not this correction's to rewrite: it belongs to the profiling rung that owns the arm path, and it changes what `G23a` measures.

**RESOLVED 2026-08-08 — the owner took neither lever: the budget rose to 1 280 KiB and no source changed.** Both remain available if a measurement later says the *committed* column is too high. This row stands at 1 208.2 KiB and **no document may quote "≤ 1 MiB retail"** — the target is 1 280 KiB. And 1.18 MiB is the figure **for the profiler alone**: with `boyko_log` also present the number a shipped title pays is the **joint** one, which `seam/joint-cost` owns and which this file does not restate. (It restated it as ≈ 1.99 MiB one repair ago; that figure was assembled from a logger retail half of 1.10 MiB against the 1.15 the logger's own files carried, so it was a fourth statement of a one-owner number *and* a wrong one. The **1 208.2 KiB** above is derived in this file's own table and is what stays; the **908.2 KiB** it replaces was correct arithmetic over a `LANE_COUNT` the substrate had already deleted, which is the whole failure mode — a total can be a perfect sum of its operands and still be a stale number.) The owner-facing consequence is raised in `docs/OPEN-QUESTIONS.md`.

**Every row of this table is the ARMED figure** (S13). Each is committed by `arm`, which is the enable path; with the runtime flag off `arm` never runs, the columns are never committed, and the `.bss` column is reserved address space that no page fault has yet made resident. `profiling/00-GOAL-TARGETS.md` carries the flag-off column of the budget table and the gate (`GJ1`) that measures it.

---

## Public API — emission and session control

```rust
// ── crate partition: ONE line per crate that declares zones; no default (B3/D6) ──
boyko_diag::profiling_partition!(Engine);   // const-asserts CARGO_PKG_NAME ∈ ENGINE_PACKAGES
boyko_diag::profiling_partition!(User);     // games, plugins, mods, tools, test targets

// ── emission (above the tier ceiling / feature off: expands to NOTHING) ──
// A ZoneSite is what `declare_zone!` declares; `ZoneSite` pairs with the logging plan's
// `LogSite` and is the noun used throughout this corpus (S11).
declare_zone!(IDENT, name = "...", channel = ..., kind = ..., stage = ..., group = ...,
              scope = ..., tier = ...);     // region comes from crate::__BOYKO_ZONE_PARTITION
zone!(IDENT);                                 // RAII
#[must_use] zone_open!(IDENT) -> ZoneGuard;   // cross-function brackets
counter!(IDENT, value: u64);
gauge!(IDENT, value: u64);

// ── dynamic emission (data-defined zones; USER partition, always) ──
pub struct ZoneSpec<'a> { pub name: &'a str, pub channel: Channel, pub kind: ZoneKind,
                          pub unit: Unit, pub scope: u8 }
pub fn register_zone(spec: ZoneSpec<'_>) -> Result<DynZoneHandle, RegisterError>;
zone_dyn!(handle);  counter_dyn!(handle, v);  gauge_dyn!(handle, v);
pub fn zone_dyn_open(h: DynZoneHandle) -> u64;      // FFI/script seam: returns an opaque token
pub fn zone_dyn_close(h: DynZoneHandle, token: u64);

// ── lanes: OWNED BY boyko_diag (S3); this crate re-exports, it does not define ──
pub use boyko_diag::lane::{lane, claim_lane, release_lane, LANE_COUNT, LANE_UNCLAIMED};

// ── session control ──
pub struct ProfilerConfig {           // NOTE: `window` is NOT a field (F25) — WINDOW is a const
    pub scopes: u64,                  // initial ARM_MASK
    pub user_zone_budget: u16,        // 0..=MAX_USER_BUDGET; fixes zone_stride for the session
                                      //   (rev 3's `dyn_zone_budget`: it now also covers a
                                      //    user crate's STATIC zones — B3)
    pub hist_slots: u16,
    pub analysed_schedule: ScheduleLabel,   // which schedule's ConflictGraph is snapshotted (D9)
    pub telemetry: Option<TelemetryConfig>, // .quantiles: &[ZoneId], ≤ 64 (M7)
}
pub fn arm(world: &mut EcsMaster, cfg: ProfilerConfig) -> Result<(), ProfilerError>;
pub fn disarm(world: &mut EcsMaster);   // a mask store; frees nothing (D15)
// re-arm with a different geometry => E9213

// ── reading — kind-specific, so the wrong statistic is unreachable (D13) ──
impl Profiler {
    pub fn span(&self, id: ZoneId)    -> Option<SpanWindow<'_>>;
    pub fn counter(&self, id: ZoneId) -> Option<CounterWindow<'_>>;
    pub fn gauge(&self, id: ZoneId)   -> Option<GaugeWindow<'_>>;
    pub fn lifetime(&self, id: ZoneId)-> Option<LifetimeAcc>;      // retention tier B (D22)
    pub fn histogram(&self, id: ZoneId)-> Option<HistView<'_>>;    // tier C; quantiles as EDGES
    pub fn by_name(&self, name: &str) -> Option<ZoneId>;           // #[cold], setup / reducer only
    pub fn frame(&self, back: u32)    -> Option<&FrameRecord>;     // 0 = newest SEALED
    pub fn rounds(&self, back: u32)   -> &[RoundRecord];
    #[cfg(feature = "profiling-analysis")]
    pub fn concurrency(&self)         -> ConcurrencyReport<'_>;    // declared vs observed (D9)
    pub fn quantum(&self, ch: Channel)-> Quantum;                  // Known(u64) | Unknown (S7)
    pub fn drops(&self)               -> DropCounters;
    pub fn clock(&self)               -> ClockCalibration;
    pub fn zone_tier(&self)           -> ZoneTier;                 // vs retention_tier (S11)
    pub fn clock_epoch(&self)         -> u32;                      // boyko_diag::clock's (S4)
    pub fn reserved_bytes(&self)      -> usize;                    // VM_LEN; G23a/G23b's domain 2 (M10)
    pub fn latency(&self)             -> LatencyTable;             // the published table (D25)
}

// ── diagnostics seam: the single site the logging plan re-points ──
pub(crate) fn emit_diag(code: DiagCode, fields: &[(&'static str, DiagValue)]);  // #[cold]
```

The kind-specific window types (`SpanWindow`/`CounterWindow`/`GaugeWindow`), the contrast surface (`Floor`/`Twin`/`resolve`/`ContrastPlan`) and the game-facing surface (`register_scope`, `ProfiledZone`, `flush_on_panic`, the artifact writer) are declared by `profiling/03-STATISTICS.md` and `profiling/04-GAME-FACING.md`. The **absence list below is carried here in full** because it constrains all three files at once and a copy in each is a copy that can drift:

**Deliberately absent:** any function returning a bare delta · any ns value without its `calib_cv` · any GPU reader that can block · any accessor that panics on the wrong `ZoneKind` · **any `Floor` constructor taking a sigma or a single sitting** · **any public `ARM_MASK` setter** · any `&str`-keyed emission · any point-estimate quantile from a histogram.

---

## Algorithms

### A1 — `ZoneGuard::open` / `Drop`

```
open:  0. const { $h::TIER <= GLOBAL_TIER }         -- compile-time; false => nothing is emitted
          (from the `mod` companion; through the handle it is E0080 -- see declare_zone!)
       1. ARM_MASK.load(Acquire) -> bt scope_bit; not taken -> NULL guard, return      (D1/F11)
       2. HANDLE.id.load(Relaxed); UNASSIGNED/RESERVED -> #[cold] register (D6)
       3. rdtsc
       4. guard = { stamp, id, lane }        // lane = boyko_diag::lane(), ONE load (D2/S3)
drop:  5. id == DISABLED || lane == LANE_UNCLAIMED -> #[cold] count and return
       6. rdtsc; value = now - stamp        // u64: no saturation test, no Extension record (B1)
       7. reg = &LANES[lane].<REGION>;  buf = reg.w.buf.load(Acquire); idx = reg.w.write.load(Relaxed)
       8. idx - reg.r.read.load(Acquire) >= REGION_CAPACITY -> #[cold] overflow.fetch_add(1); return
       9. store 24 B at buf[idx & MASK]     // one 16 B + one 8 B store
      10. reg.w.write.store(idx + 1, Release)
```

`REGION` is a **compile-time constant of the declaring crate** (`crate::__BOYKO_ZONE_PARTITION` — B3/D6), not of the macro, so step 7 is still an immediate offset and not a branch.

Complexity O(1); ~5 instructions + 2 `rdtsc` armed (one fewer than rev 3: the `d > u32::MAX` compare and its branch are gone), 1 load + 1 predicted branch disarmed. Cache: a monotone cursor, **2.67 samples per line, ~0.375 line touches/sample**, write-allocated, with 2 of every 8 stores straddling a line boundary (D1's re-derivation). **No non-temporal store** — the fold reads these bytes within one frame, so evicting them is strictly worse. **No software prefetch** — the hardware stride prefetcher already covers a monotone cursor. Branches: two, both `#[cold]`-biased. SIMD: none wanted (24 B is two stores, and a 32 B AVX store would waste 8 B of ring per record).

**Step 1 is the runtime axis, and step 0 is the compile axis** (S13). Step 1 is what a shipped binary can be asked to turn on; it costs one `.bss` load plus one statically-predicted branch and **cannot be driven below that** by any runtime mechanism. Step 0 is what deletes steps 1-10 and their operands entirely.

**Why `buf` cannot be null at step 9** (F11): `arm()` stores the slab pointers `Release` **before** it stores `ARM_MASK` `Release`; step 1 is an `Acquire` load, so observing a set mask happens-after the pointer publication. A `debug_assert!(!buf.is_null())` records the invariant, and a loom case exercises it.

### A2 — Fold (top of `App::update_with_delta`, for CLOSED frames only)

Runs **before** step ① `Time::advance_with` (`crates/boyko_ecs/src/ecs/core/app/app.rs:655-676`) — the single funnel both `App::update` (`:736`) and the host's `app.update_with_delta(dt)` (`crates/boyko_app/src/runner.rs:1321`) pass through (F2).

```
0. scope projection (D20): for b in 0..scope_count:
       bits |= (world.is_enabled::<ProfilingScopeEnabled>(scope_entity[b]) as u64) << b
   if bits != ARM_MASK.load(Relaxed) { ARM_MASK.store(bits, Release) }
1. if ARM_MASK == 0 { return }                       // the disarmed cost: 1 load + 1 branch
2. clock check: if elapsed since last fold > MAX_PLAUSIBLE_FRAME_TICKS ->
       #[cold] boyko_diag::clock::note_forward_jump(); discard the in-flight window;
               drops.clock_epoch_breaks += 1; W9216; calibrate()
3. cut = frame_begin_tsc[current]        // samples at or after `cut` belong to the live frame
   for lane in 0..LANE_COUNT, for reg in [Engine, User]:
       w = reg.w.write.load(Acquire)     // publishes every sample byte below w
       r = reg.r.read.load(Relaxed)      // the dispatcher is the sole consumer
       for i in r..w:
           s = buf[i & MASK]
           if s.stamp >= cut { stop this region }        // SAME field for every kind (B1)
           f = walk(frame_begin_tsc, f_prev, s.stamp)    // bidirectional; see "disorder" below
           if f is older than the retained window { drops.late += 1; continue }
           match kind:                                   // dispatch AFTER attribution, never before
             Span    -> total[f*Z+z] += s.value; count[f*Z+z] += 1
                        min/max from clamp(s.value, u32);  if clamped -> label = OVER_RANGE,
                                                           drops.span_over_range += 1
                        if sys_of[z] != NONE -> intervals APPEND {s.stamp, s.value, sys, occ} (F19b)
             Counter -> total[f*Z+z] += s.value; count[f*Z+z] += 1   // per-frame SUM (a rate)
             Gauge   -> total[f*Z+z]  = s.value; min/max fold        // last-write-wins level
           if hist_of[z] != 0 { hist_fold(z, s.value) }  // A9, retention tier C
       drops.<region>_overflow += overflow_since(lane, region, &mut seen[lane][region])  // Q2(b)
       reg.r.read.store(w, Release)
4. lifetime_fold(row = current frame row)   // ONE sequential pass, row still in L1d (D22 tier B)
5. if window boundary && telemetry_armed { __telemetry_reduce(); __telemetry_write() }  // D23/A10
```

**Attribution reads `stamp` and only `stamp`, for all three kinds, before the kind dispatch (B1).** Rev 3's cut test and frame walk consumed `begin`, which was a payload for two of the three kinds — so counters landed in `drops.late` and large gauge values truncated the region's fold. The rule is now structural rather than ordering-dependent: **no field whose meaning varies by kind may be read before the `match`**, and `stamp` is the only field whose meaning does not.

**A counter's per-frame cell ACCUMULATES; a gauge's is last-write-wins.** Rev 3's `Counter -> total[f*Z+z] = s.begin (level)` was an assignment, which cannot support `CounterWindow::rate_per_frame` — a rate needs the frame's sum. `level()` is served from the retention-tier-B lifetime accumulator instead, where a running total belongs.

**Disorder, and why the walk is bidirectional.** A region is *not* TSC-monotone, and rev 3's "O(1) amortised: a region is TSC-monotone" was false for the very case D3a designs for: a `Span` stamps at **open** and is written at **close**, so a nested pair writes the inner span (later stamp) before the outer (earlier stamp). The walk therefore keeps the previous frame index `f_prev` and moves **both ways**, bounded by the retained window; a stamp older than the window is `drops.late`. Amortised cost is unchanged in the common case (consecutive samples are in the same frame or the next one); the worst case is one excursion per nesting level, bounded by nesting depth, which `OPEN_DEPTH` already bounds in debug.

**What the stop rule costs, stated.** Because the SPSC cursor can only publish a *prefix* as consumed, the region stops at the first sample with `stamp >= cut`, and a long-running outer span written after a short inner one that opened past the cut is deferred to the next fold. It is not lost — it is attributed by its own `stamp` when it is folded, one fold later — and if it ages past the window it becomes `drops.late`. In the windowed host the cut rarely fires at all: the fold runs at the top of `update_with_delta`, outside the schedule, when no worker is running; it matters for threads that claimed a lane and emit concurrently, which is a supported configuration.

- Complexity O(samples); ~400/frame → **9.6 KiB read** (24 B records); 21 B × `Z` written per frame row (21 KiB at `Z = 1024`, D8/M9) ⇒ **30.6 KiB against a 32 KiB L1d**.
- Cache: region reads strictly sequential; column writes scattered **inside one row per column**. The lifetime pass and the histogram fold hit lines the sample loop just touched.
- Branching: one 3-way jump table on `kind`, the cut test, the walk's direction test, and a `hist_of[z] != 0` byte test.
- **Sealing:** a frame becomes `Sealed` when its fold completes **and** (`GpuPass` disarmed **or** its GPU slot retired). If neither holds after `GPU_RING_DEPTH + RETIRE_GRACE_FRAMES` frames it becomes `Partial`. So `frame(0)` is never permanently `None` with the GPU channel off.
- **Step 0's cost is ≤ 5 ns × `scope_count`** (`crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:100-105`) and is inside `__fold`, hence inside `instrument_measured` and outside `__frame` (D16).

### A7 — `register_zone` (dynamic, `#[cold]`, lock-free, allocator-free)

```
1. validate: spec.scope >= 32 else W9212, Err(EngineScopeRefused)
             spec.name.len() <= MAX_ZONE_NAME_BYTES else truncate + flag
2. id = USER_ID_NEXT.fetch_add(1, Relaxed)    // the SAME counter a User-crate declare_zone!
                                             // draws from (B3/D6) -- one budget, one range
   if id >= armed_user_budget {
       USER_ID_NEXT.fetch_sub(1, Relaxed);       // monotone bound restored; no id leaked
       drops.user_registrations_refused += 1; W9210; return Err(BudgetExhausted) }
3. off = DYN_NAME_NEXT.fetch_add(padded_len, Relaxed)
   if off + padded_len > DYN_NAME_BYTES { fetch_sub; W9210; return Err(NameArenaExhausted) }
4. copy name bytes into DYN_NAMES[off..]; build &'static str from the reserved range
5. write ZoneDesc into DYN_DESCS[id]              // sole writer: this thread's reserved slot
6. REGISTRY[ENGINE_ZONE_SLOTS + id].store(desc_ptr, Release)      // THE publication edge
7. return DynZoneHandle { id: ENGINE_ZONE_SLOTS + id, arm_bit: 1 << spec.scope }
```

O(name_len). No CAS loop, no spin, no allocation, no lock. **Steps 5→6 are the ordering contract:** the desc is fully written before the pointer is published, so any `Acquire` reader of `REGISTRY[i]` sees an initialised desc; and the handle — hence the ability to emit — is only returned after step 6, so a `Sample` can never carry an id whose desc is unpublished. The `fetch_add`/`fetch_sub` reservation can transiently over-report but never under-report, and every claimant re-checks the bound.

### A9 — `hist_fold` (fold inner loop, retention tier C)

```
z   = d.clamp(1 << 6, (1 << 30) - 1)      // two branchless clamps
e   = 63 - z.leading_zeros()              // lzcnt
m   = (z >> (e - 3)) & 7
idx = ((e - 6) << 3) | m                  // 0..191
slot.buckets[idx] = slot.buckets[idx].saturating_add(1)
slot.total += d as u64; slot.count += 1
```

~8 instructions, no branch except the saturating add's (`adc`/`cmov`-shaped). Executed only for zones with a slot. Measured budget: `fold_cost` +18 % at 64 active slots. Saturation is counted (`hist_saturations`), never silent.

---

## Multithreading model

This table is **one object with one ordering rationale**, and it is carried whole here even for the data whose decision lives in `profiling/02-GPU.md` (the `FrameSlot` rows, D5). Splitting it is how two files come to disagree about a memory ordering.

| Datum | Sharing | Writer | Reader |
|---|---|---|---|
| `ARM_MASK` | shared, read-mostly, `CachePadded` | the fold's scope projection, `arm`/`disarm` | every emitter, **`Acquire`** |
| `LANES[n].<reg>.w.buf` | shared pointer | `arm` at **first arm only** (`Release`) | region owner (`Acquire`) |
| `LANES[n].<reg>` sample bytes | **single writer** = lane owner | lane owner | fold |
| `LANES[n].<reg>.w.write` | 1W/1R | lane owner (`Release`) | fold (`Acquire`) |
| `LANES[n].<reg>.r.read` | 1W/1R | fold (`Release`) | lane owner (`Acquire`) |
| `LANES[n].<reg>.w.overflow` | shared counter, **monotone** | lane owner (`fetch_add`, `Relaxed`) | fold (`load`, `Relaxed`; the delta lives at the consumer — Q2(b)) |
| `REGISTRY[i]` | shared | first executor / `register_zone` (`Release`) | fold, window reducer (`Acquire`) |
| `ZoneHandle.id` | shared | CAS `UNASSIGNED→RESERVED`, then store (`Release`) | emitters, **`Relaxed`** |
| `ENGINE_ID_NEXT` / `USER_ID_NEXT` / `DYN_NAME_NEXT` | shared counters | any registrant (`fetch_add`/`fetch_sub`, `Relaxed`) | same |
| `DYN_DESCS[k]` / `DYN_NAMES[..]` | **single writer per reserved range** | the thread whose `fetch_add` reserved it | anyone, gated by `REGISTRY`'s `Release` |
| `boyko_diag::lane::LANE` | thread-local, **no `Drop`** (S3) | the owning thread, once (D2) | the owning thread, and the logger |
| `Profiler` | dispatcher/host-only for mutation | fold / retire / window reducer | `Res<Profiler>` systems |
| `FrameSlot.marks` | 1W/1R | recorder (plain stores) | retire, gated by `seal` |
| `FrameSlot.seal` | 1W/1R | recorder (`Release`) | retire (`Acquire`) |
| telemetry encode buffers (`.bss` static, S5) | dispatcher-only | dispatcher, and `flush_on_panic` on the panicking thread | `write_all` (OS buffer — named FFI exception) |
| `VM_BASE` / `VM_LEN` | shared, write-once | first `arm` (`Release`) | `Profiler`, `G23a/G23b` (`Acquire`) |
| `boyko_diag::clock` `TICKS_PER_NS` / `CLOCK_EPOCH` | shared, read-mostly | `calibrate` / `note_forward_jump` (`Release`) | profiler fold **and** logger sink (`Acquire`) |
| `boyko_diag::loss` `LossCell` | lane-owned, no lock prefix on the owner's path | the lane owner | its subsystem's consumer, via `fold_into` |

**Ordering, each justified.**

- **`ARM_MASK` `Acquire` load / `Release` store.** It gates the lane `buf` pointer, which is published before it. **On x86-64 an `Acquire` load of an aligned word is a plain `mov`** — zero extra instructions — so rev 2's "Relaxed because Acquire would cost a fence off x86" was backwards *and* missed the obligation (F11). The scope projection's store is `Release` for the same reason. A worker seeing a stale mask for one frame records or skips one frame's samples, which is not a correctness property (G12 asserts the *next* frame).
- **`w.buf` `Release`/`Acquire`** — the slab's initialisation must happen-before any write through the pointer; the only pointer-carrying edge on the transport side.
- **`w.write` `Release`/`Acquire`** — the sole publication edge for sample bytes, the same edge `EventBuffer::write_len` uses (`crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:79-81`).
- **`r.read` `Release`/`Acquire`** — publishes "these slots are reusable" before the producer may overwrite.
- **`overflow` `Relaxed` both sides** — a counter with no ordering obligation, and **monotone**: the consumer never writes it, so no clear can race an increment at all (D24a, applying `substrate/loss-fold`'s Q2 resolution (b)).
- **`ZoneHandle.id` `Release` store / `Relaxed` load, `REGISTRY[i]` `Release`/`Acquire`** (F10, one specification only). The emitter's `Relaxed` id load is sound because **it never dereferences a desc** — it stores a bare `u16`. The desc edge is carried entirely by `REGISTRY[i]`: a fold that `Acquire`-loads the value the registrant `Release`-stored synchronises-with it, and every desc byte was written before that store (A7 step 5 → 6, D6 step 4 → 5). This holds whether or not the emitter is the registrant.
- **`ENGINE_ID_NEXT` / `USER_ID_NEXT` / `DYN_NAME_NEXT` `Relaxed`** — monotone reservation counters carrying no data; the data edge is the `REGISTRY` store.
- **`VM_BASE` `Release` / `Acquire`** — the reservation's bytes are committed before the base is published; every column pointer is derived from an `Acquire` load of it.
- **`boyko_diag::clock`'s epoch `Release`/`Acquire`** — `note_forward_jump` publishes the bumped epoch before either consumer stamps another record, so a straddling record is legible on both sides (S4).
- **`FrameSlot.seal` `Release`/`Acquire`** — one edge for the whole mark array, which is why no 128-bit atomic is needed (D5).
- **No `SeqCst` anywhere.**

**Data-race freedom.**

1. *Sample bytes.* Exactly one writer per **region** by construction (D2 + D19): workers write `LANES[worker_id]`, the dispatcher `LANES[LANE_DISPATCHER]`, the host thread its claimed lane, unclaimed threads nothing. Within a lane the region is a compile-time constant of the **declaring crate**, so one thread's engine and user regions are two independent SPSC rings with the same producer. One OS thread holding two lane identities over its life is serial, so each region still has one writer. Producer touches `[write, read + CAP)`; consumer touches `[read, write)`; disjoint given A1 step 8. Textbook Lamport SPSC — no CAS, no ABA (monotone `u32` cursors, masked indexing).
2. *Cursor wrap* (F23, rev 2's "49 days" was 24× wrong). `u32` wraps after 4.295 G samples: at 400 samples/frame/60 Hz that is **≈ 49.7 hours**; at a hot game lane's 2000/frame it is **≈ 9.9 hours**. Both are reachable in a soak or a long session, so correctness across one wrap is required, not incidental — the masked-index + unsigned-difference form provides it, and a unit test drives the cursor across `u32::MAX`.
3. *Overflow-counter wrap.* Impossible between two folds by D24c's arithmetic; accumulated into `u64`.
4. *False sharing.* `ZoneLane` = 256 B with **four** distinct lines, pinned by `offset_of!` const-asserts; `ARM_MASK` `CachePadded`; lanes 256 B apart.
5. *Dynamic descriptor arenas.* Each slot has exactly one writer (the `fetch_add` reserver) and is never reused or freed, so no reader can observe a torn re-initialisation; every read path goes through the `REGISTRY` `Release`/`Acquire` edge. `SyncCells` carries a manual `unsafe impl Sync` with these two clauses spelled out (`substrate/never-freed-storage`).
6. *Store.* `Profiler` is mutated only outside the schedule (fold, retire, window reducer — D16/A3), so every in-frame `Res<Profiler>` reader sees one consistent snapshot. **No new synchronisation primitive is introduced.**
7. *Scope projection.* The fold reads the world through `&mut EcsMaster` at a point where no system is running; the `ARM_MASK` store is the only cross-thread effect and carries no data.
8. *Teardown.* There is none, and after B4 that is **structural**: the reservation is `mem::forget`-ed at first arm (D8), so no `Drop` exists that could unmap it — not the `Profiler`'s, not the world's, not a multi-world test's. `buf` is never nulled, `DYN_DESCS` slots are never reused. `is_in_system_run()` is used **only** as a same-thread setup assertion (`crates/boyko_threadpool/src/tls.rs:83` reads the calling thread's TLS), never as a cross-thread barrier.
9. *`Send`/`Sync`.* **`Profiler`: manual `unsafe impl Send + Sync`** with D8's three clauses (mutation only outside the schedule; in-frame access shared-only through `Res<Profiler>`; a write-once base into a region that is never resized, moved or freed) — required because a `NonNull<u8>` field is `!Send`/`!Sync` while `Resource: Send + Sync` (`resources/resource.rs:42`), and **absent from rev 3 entirely**. `ZoneLane`: manual `unsafe impl Sync`, three clauses adapted from `ThreadLaneWriter` (`event_buffer.rs:93-110`) — (a) single writer per region, enforced by D2's per-thread lane and D19's const region; (b) the consumer touches only `[read, write)`; (c) the atomics cover the cursors, and the sample bytes are covered by the `write` `Release`/`Acquire` edge — **not** by a `&mut` synchronisation point, because a `static` has none (this is why rev 2's `EventBuffer` analogy was withdrawn, F7). `FrameSlot`: `Sync` by single-producer + `seal`. `ZoneGuard` is `!Send` via `PhantomData<*const ()>` — it carries a lane index bound to the current thread. `DynZoneHandle` **is** `Send + Sync + Copy` — 16 B of plain data with no thread affinity, which is what lets a game store it in a component and emit from any lane.
10. *Panic.* `ZoneGuard::drop` runs during unwind, so a panicking system's zone closes. Moot in practice: `crates/boyko_threadpool/src/worker.rs:157-168` aborts on worker panic. `flush_on_panic()` is called by the logging plan's single process-global hook and touches only host-owned state (`seam/lifecycle-order`).

**Partitioning.** CPU zones partition by lane **and** by region (no stealing, no redistribution, no contention). GPU zones partition by frame slot; exactly one thread touches a slot at a time. The window reducer and the telemetry writer are single-threaded and `#[cold]`. **Rev 4 adds no thread** — and, with `boyko_diag`'s shared lane registry, it removes one TLS slot and one `Drop` guard from the joint configuration (S3).
