# Profiling — goal, audiences, budgets, invariants, environment

<!-- CONTRACT
provides: profiling/goal-and-audiences
provides: profiling/budgets-and-invariants
assumes: substrate/dedup-rationale
assumes: substrate/crate-graph
assumes: substrate/lane-registry
assumes: substrate/mute-leaf-rule
assumes: substrate/never-freed-storage
assumes: substrate/section-report
assumes: seam/decisions-s1-s12
assumes: seam/build-axis
assumes: seam/free-when-off
assumes: seam/joint-cost
assumes: seam/lifecycle-order
assumes: seam/open-owner-calls
assumes: seam/vocabulary
-->

*Carved from `docs/PROFILING-SYSTEM-PLAN.md` (rev 4) — its preamble, `Supersedes`, `Module paths`, §"Goal, and the two audiences", §"Where the two audiences conflict", §"Performance budgets", and §"Context and constraints" (the crate graph, Principle 0, Invariants 1-7, and the hard environmental constraints). Diff against the monolith until it is retired.*

---

## Status and provenance

**Status:** design, pre-implementation. **Revision:** rev 4. The design folds **four** inputs into one:

1. the second-pass architecture review of rev 2 — verdict **REJECTED**, 8 BLOCKER / 13 MAJOR / 7 MINOR (`F1`…`F28`);
2. an owner-stated **scope extension**: the profiler is used not only to evaluate the engine but **by the games themselves** — collect as much data as possible, be maximally flexible (`X1`…`X25`);
3. the third-pass review of rev 3 — verdict **REJECTED**, 6 BLOCKER / 7 MAJOR (`B1`…`B6`, `M7`…`M13`);
4. the **seam decision record** for `boyko_diag`, the shared diagnostics substrate this plan and the logging plan both stand on (`S1`…`S12`) — the two plans were judged **INCOMPATIBLE AS WRITTEN** and the record's decisions are implemented, not re-litigated. They live once, in `SEAM.md`.

A fifth input arrived when this corpus was split: **S13, the owner's free-when-off requirement**. It is specified in `SEAM.md` (`seam/free-when-off`) and its consequences for this file's numbers are marked below.

**Silence on a finding is itself a defect**, so every `F`, `X`, `B`, `M` and `S` has a row. The disposition tables are in `profiling/06-DISPOSITIONS.md`; the `S` rows live in `SEAM.md`.

**Supersedes:**

- `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` (all three collectors);
- the `BOYKO_VB_BENCH` / `BOYKO_SV0_BENCH` harness bodies and the private statistics helpers in `crates/boyko_app/src/runner.rs` — the producers of the `VB-P1d …` / `VB-P4 pass=…` / `VB-P4 regime …` stdout lines (`runner.rs:3089`, `:3096`, `:3121`, `:3137`);
- **and therefore the stdout measurement channel itself.** Six files consume those lines today and all six migrate to the artifact in one commit at rung 7 (S1): `crates/boyko_app/tests/vg_occ_split_timing.rs`, `vb_bench_totality_gate.rs`, `vb_bench_query_validation.rs`, `vg_decidability_floor.rs`, `vb_p1d_cull_shade_bench.rs`, `sv0_deferred_term_bench.rs`. `vg_decidability_floor.rs` is decisive rather than incidental: it parses the shipped bench's own stdout (`:133-160`, *"Parsing the shipped bench's own output"*) and it is the instrument that produces the `Floor` this plan's own band consumes. **Rung 7 therefore breaks rung 8's input, and every published floor number is invalidated until rung 7b re-measures it** — enforced with no new mechanism, because the new channel carries a new `WorkloadTag` and `resolve` already refuses a `Floor` whose tag does not match (`FloorWorkloadMismatch`).

**Module paths, stated once** (rev 2 gave three, all wrong — F24; rev 3's leaf moved again in rev 4 — S2):

| Half | Path | Crate |
|---|---|---|
| Emission ABI: vocabulary, registry, transport, macros | `boyko_diag::profiling_abi` | `boyko_diag` (the new zero-dependency bottom; **not** `boyko_utils`, which keeps its empty `[dependencies]`) |
| Clock, lane identity, loss vocabulary, never-freed-storage policy | `boyko_diag::{clock, lane, loss, storage}` | `boyko_diag` — **shared with the logging plan**, one owner each |
| Store, statistics, contrast, ECS control surface, the fold | `boyko_ecs::ecs::core::profiling` | `boyko-ecs` |
| GPU recorder | `boyko_rhi_vulkan::present::gpu_zone` | `boyko_rhi_vulkan` |
| Host retire + window reducer + artifact + stream | `boyko_render::profiling_bridge`, `boyko_app::profiling::{reduce,artifact,stream}` | — |

---

## Goal, and the two audiences

Replace three hard-coded GPU timestamp enums, two env-var bench harnesses and ~600 lines of hand-rolled statistics with **one ECS-native measurement subsystem** that answers six questions and structurally refuses to answer a seventh.

| Question | Audience | Channel / scope | Primary statistic |
|---|---|---|---|
| What did system `S` cost on the CPU this frame? | engine | `SchedulerCpu` | median / p95 over a `WINDOW`-frame window |
| Which systems actually ran concurrently? | engine | `SchedulerCpu` + retained intervals | **static compatibility** (from `ConflictGraph`) vs **observed interval overlap** |
| What did GPU pass `P` cost? | engine | `GpuPass` | median / p95 device ticks, per-pair `MEASURED / NOT_BRACKETED / TORN / LOST` |
| How many draws / culled instances / bytes? | both | `Counter` / `Gauge` | rate-per-frame (counter) or level (gauge) — typed, never interchangeable |
| Where did the frame go? | both | `Frame` | two lanes (CPU TSC, GPU device ticks), `cpu_gpu_offset = UNCORRELATED` in v1 |
| **What did this *player's* session cost, over hours?** | **game** | lifetime accumulators + histograms + a window-granular stream | session count / total / max, log-linear quantiles |
| Is `A` faster than `B` by `Δ`? | engine | contrast API | `Resolved { … }` **or** `NotResolved { reason, … }` — no third return, no bare-delta constructor |

The seventh question — *"just give me the delta"* — is **structurally unanswerable**: `resolve` returns `Resolved{..}` or `NotResolved{reason}`, there is no third variant, and there is no bare-delta constructor anywhere in the API.

### Where the two audiences conflict — named, decided, and costed

A game-facing profiler and an engine-measurement profiler want opposite things in six places. Each is decided here, with the cost to the side that loses.

| # | Conflict | Decision | Cost to the losing side |
|---|---|---|---|
| **C-I** | Fixed-capacity lossy rings (what buys 12 ns emission) vs "collect as much as possible" over an hours-long session | Ring stays fixed and lossy. Hours are served by **lifetime accumulators** (retention tier B) + **log-linear histograms** (tier C) + a **window-granular binary stream** (D23) | **Game:** no per-frame history beyond `WINDOW` frames, ever. An hour of per-frame rows must be reduced offline from the stream |
| **C-II** | Static `declare_zone!` handles (0-instruction disabled path, tier-foldable) vs zones defined by data / config / script / mods | Both, in a **partitioned** id space and a **partitioned** ring region — and the partition is keyed on the **declaring crate**, not on which macro was used (D19, B3) | **Game:** a dynamic zone costs ≤ 14 ns (≤ 18 ns across FFI) instead of ≤ 12, cannot be compile-time tier-folded, and is refused past `user_zone_budget`. **Engine:** every crate that declares a zone must state its partition once at its crate root, or it does not compile |
| **C-III** | Terminal exhaustion (`E9201`, a loud engine diagnostic) vs a shipping title that must never panic on a diagnostic | **Non-terminal everywhere.** Exhaustion yields `ZoneId::DISABLED`, a counted refusal and a once-per-session warning | **Engine:** a mis-sized budget now shows up as a `zones_refused` field instead of a crash. G11 is the gate that makes the field non-vacuous. This is a deliberate reversal of rev 2 (F5) |
| **C-IV** | "The instrument is outside its own primary number" (D16) vs a game reading its own counters *while the frame runs* | Reads are **windowed and lagged**: CPU data is frame N−1, GPU is N−4…N−2 (D25) | **Game:** no same-frame readback. LOD / dynamic-resolution decisions consume N−1. Same-frame counter readback as a message bus is **refused** (X14) |
| **C-V** | Contrast strictness (a bench must refuse a truncated window) vs a game whose rings drop routinely | `resolve` refuses any leg whose window carried a drop or crossed a clock epoch: `NotResolved { reason }` | **Engine bench:** a bench that drops now produces *no number* instead of a wrong one. This makes the engine side **stricter**, not looser |
| **C-VI** | `profiling` default-on so shipped code carries the sites vs "zero overhead in a shipping game" | Feature default-on; the tier is one column of the **single `BOYKO_PROFILE` build axis** shared with the logging plan (`Always` / `Dev` / `Deep`), read by exactly one `build.rs` at the bottom of the graph (D21 / S9, `seam/build-axis`) | **Engine:** changing the profile rebuilds the workspace from `boyko_diag` up, ~12 KiB of dead `.bss` remains for folded handles, and CI grows from 1 to 5 full-workspace legs (4 net new, shared with logging) |

**C-VI is where S13 lands.** The compile axis of C-VI is the only mechanism that reaches literal zero per site; the runtime flag that S13 adds is the only mechanism that can be turned on in a binary that has already shipped. `SEAM.md`'s `seam/free-when-off` keeps them apart and states what each costs; this file's budget table carries the numbers.

---

## Performance budgets — three configurations, all **measured**, none asserted

("budget", not "target": `LogTarget` is the logging plan's sink type and the word is spent — S11, `seam/vocabulary`.)

| Metric | Dev, armed | Dev, disarmed | Retail (`Always` tier) | Proved by |
|---|---|---|---|---|
| static zone open+close | ≤ 12 ns median | ≤ 2 ns | ≤ 2 ns | criterion `zone_cost`, eight legs one sitting; regression gate at +25 % **against a baseline whose `config_tag` matches the sitting** (S10) |
| dynamic zone open+close | ≤ 14 ns | ≤ 3 ns | n/a (`Deep`-folded) | `zone_cost` dyn legs (G17) |
| zone at a tier above `GLOBAL_TIER` | **0 instructions** | 0 | 0 | **two-sided token-level expansion test across two ceilings** + a behavioural liveness clause (G1, G14) — *not* a per-binary recorder-symbol census, which cannot attribute a reference to a site (B5) |
| fold, armed | ≤ 5 µs/frame at 400 samples, `zone_stride ≤ 1280` | 1 load + 1 branch | ≤ 2 µs (no analysis columns) | criterion `fold_cost`, 4 legs; `__fold` is itself a zone |
| allocations/frame while armed | 0 | 0 | 0 | zero-alloc gate (existing 19-test pattern) |
| profiling `vkCmd*` recorded while disarmed | **0** | **0** | **0** | command census, two-sided **with an equality on the armed side** (G5) |
| resident, armed (`Z = 1024`) | ≤ 7 MiB (computed **6.67**) | ≤ 7 MiB after first arm (D15) | **≤ 1 MiB (profiler-attributable only — see the joint row and the two corrections below)** (computed **0.89**) | boot-residency gate over **three** domains — the counting allocator, `Profiler::reserved_bytes()` (`VmReservation::os_len()`; `reserved_bytes()` **does not exist in the tree**), and `boyko_diag::section_report` for `.bss` — two-sided, plus "a second `arm` allocates 0" (G23) |
| resident, armed, `profiling-analysis` on | ≤ 7.5 MiB (computed 7.05) | — | n/a | same gate |
| resident, `user_zone_budget = 3072` | ≤ 23 MiB, **and `W9211` fires** (computed 22.1) | — | n/a | same gate; the ceiling is the *declared* budget's, not a promise the game will not ask for it |
| **joint resident, profiler + logger both present** | **owned by `seam/joint-cost` — not restated here** | — | **owned by `seam/joint-cost` — not restated here** | `SEAM.md`'s joint cost table is the single owner and this file now carries **no** joint figure at all. Rev 4 of this row did carry one — **≈ 9.35 MiB dev / ≈ 1.99 MiB retail**, built by quoting the logger's halves as **2.68 / 1.10 MiB** — and both halves are contradicted by the logger's own files and by `SEAM.md`, which state **2.90 / 1.15**. The row said in the same breath that it "quotes and does not re-derive"; it re-derived. What this file owns is the profiler's **own** column, and that is stated as arithmetic a reader can check rather than a number a reader must trust: shipping `54 + 192 + 636 + 6.8 + 11.4 + 8 KiB = 908.2 KiB = 0.89 MiB`; dev `284 KiB + 3.75 MiB + 2.48 MiB + 52 + 108 + 8 KiB = 6.67 MiB` (both rows computed field by field in `profiling/01-EMISSION-STORAGE.md` §Sizing). **The ≤ 1 MiB row above is profiler-attributable only**; a shipping title that also boots the logger pays the joint figure, which is the larger number and the one the owner is asked about. Owner call open (S10, `seam/open-owner-calls`) |
| **joint hot-path working set** | **7-8 cache lines**, 1 TLS slot, 3 `rdtsc` per {zone + log record} | — | same | the profiler's own share is 3-4 lines (`ARM_MASK`, the `ZoneLane` writer line, the sample tail, the TLS line); the logger's is ≤ 4. **Neither plan may quote its isolated figure as the shipped one** |
| GPU readback blocking | **never** | never | never | `VK_QUERY_RESULT_WAIT_BIT` is **unrepresentable** in the new verb — a `const _` assert, i.e. a *compile* error (G2a) |
| telemetry window **total** (reduce + encode + write) | p95 ≤ 350 µs per 2 s window | — | same | criterion `telemetry_window`, three legs reported separately (M7). This is a **2.1 % spike on one frame in 121**, below this box's own decidability floor (4.7-14.3 %) — not a sustained cost, and stated as a spike rather than amortised into a per-frame average |
| telemetry encode + write alone | p95 ≤ 200 µs | — | same | criterion `stream_encode` |

### Two corrections the "≤ 1 MiB" headline must carry

The retail headline is quoted often enough that both of its qualifications travel with it, or it becomes a false claim by omission.

1. **It is profiler-attributable only, and it is false for the configuration a title actually ships.** With `boyko_log` present the number a title carries is the **joint** one, which is owned by `seam/joint-cost` and is **deliberately not restated here** — not even as a corrected figure. The reason is that the sentence previously in this slot *did* restate it, and restated it wrongly: it gave **≈ 1.99 MiB** on the strength of a logger retail half of **1.10 MiB**, against the **1.15** the logger's own files and `SEAM.md` both carry. The joint number is owned in one place precisely because separate statements of it is how the corpus contradicted itself the first time, and a corrected fourth statement is the same defect with better arithmetic. Read it from `SEAM.md`'s joint cost table.
2. **RE-CUT by S13 (`seam/free-when-off`): every "resident" cell above is the ARMED / flag-ON column.** With the runtime flag off, the same binary's diagnostics cost is **address space, not resident memory** — `.bss` is demand-zero, so an untouched table is emitted with a virtual size and no raw data. That property holds **only if boot touches nothing**, which is why calibration, the sink thread, the panic hook and every reservation commit move onto the enable path.

| Row | Meaning of the number as written | Flag-off (same binary, never armed) |
|---|---|---|
| resident, armed (`Z = 1024`) — 6.67 MiB dev / 0.89 MiB retail | the committed reservation plus the `.bss` statics, **after** a first `arm` | ~0 resident; `.bss` extents are reserved address space, never touched |
| resident, armed, analysis on — 7.05 MiB | same | ~0 |
| resident, `user_zone_budget = 3072` — 22.1 MiB | same | ~0 |
| joint resident, dev and retail — the figure `seam/joint-cost` owns (this file states none) | same, jointly | ~0, and `seam/joint-cost` carries the flag-off column for the pair |
| **static zone open+close, "Dev, disarmed ≤ 2 ns"** | **this row does NOT re-cut.** One `.bss` load plus one predicted branch is what a runtime flag costs at every surviving site, in every frame, forever | unchanged: ≤ 2 ns. **A runtime flag cannot reach zero per-site cost.** Only the compile-time ceiling (C-VI / D21 / S9) deletes the site and its operands |

**What proves the flag-off column.** Gate **`GJ1`** (specified in `seam/free-when-off`, tabled in `profiling/05-LADDER-GATES.md`, sat at the joint baseline rung J2): the same scene run flag-ON, flag-OFF, and a **control leg** built with the const ceiling forced permissive and the runtime flags off, so every site the shipping ceiling had deleted is present and paying exactly the runtime check. **If the control leg does not resolve apart from the flag-off leg, the instrument measured nothing** and the claim is recorded as UNPROVEN rather than restated. And the memory row is *not* GJ1's to claim: `substrate/section-report` proves the bytes are absent from the image; whether the OS leaves an untouched page uncommitted is **UNPROVEN and is not asserted anywhere in this corpus**.

The one number S13 does **not** move: D15's honest consequence — *"disarmed = a few KiB of `.bss` and nothing else" is true only before the first arm; after a first arm the disarmed resident cost is the full committed reservation.* That sentence describes the DISARM path, which is a mask store over an already-committed reservation. It is unrelated to the flag-off path, where `arm` never ran. Both are true and they are about different states.

---

## Context and constraints

### The crate graph, and why the GPU half is buildable at all (F1)

Rev 2 declared the GPU zone vocabulary in `boyko_ecs` and used it inside `boyko_rhi_vulkan`. **That does not compile.** Verified against HEAD:

- `crates/boyko_rhi_vulkan/Cargo.toml:42-49` — dependencies are `boyko_rhi` and `boyko_sdf_math` only (plus `windows-sys` under `cfg(windows)`).
- `crates/boyko_rhi/Cargo.toml:7-10` — dependency is `boyko-utils` only.
- Neither can name `ZoneId`, `GpuStage`, `PartitionGroup`, `declare_zone!` or `counter!` if those live in `boyko_ecs`.
- The tree states the rule against the naive fix (add `boyko-ecs` to the backend): `crates/boyko_render/Cargo.toml:44-50` — *"the low-level `boyko_rhi_vulkan` backend must not depend upward on the scene crate."*

**Rev 3's decision was "the emission ABI moves DOWN into `boyko-utils`". Rev 4 moves it one step further down, into a NEW bottom crate `boyko_diag` (S2), for a reason rev 3 could not see: the logging plan needs the same leaf.**

The reasoning that put the ABI in a zero-dependency leaf is unchanged and was upheld by the review — `crates/boyko_utils/Cargo.toml` genuinely has an **empty `[dependencies]`**, and `type_intern` is a real precedent for a process-wide registry in the leaf. What changed is *which* leaf. `boyko_log` needs a lane identity, a clock and a loss vocabulary from below `boyko_ecs` too; putting them in `boyko_utils` would give `boyko_utils` a reason to grow diagnostics, and the seam review's measured consequence of **not** sharing is concrete: the same worker would be lane 5 to the profiler and lane 37 to the logger, so no reader could place a log line inside the zone it happened in — the one joint question the pair exists to answer (`substrate/dedup-rationale`). So:

- **`boyko_diag`** — new, `std` only, **zero workspace and zero third-party dependencies**. Owns `clock`, `lane`, `loss`, the never-freed-`storage` policy + its `section_report` gate helper, and **hosts** `profiling_abi` (hosted, not shared: the logger never names `ZoneId`).
- **`boyko_utils` keeps its empty `[dependencies]`** and gains nothing. Rev 3's `boyko_utils::profiling_abi` module is withdrawn.

```
boyko_diag  (std only, zero deps)  ── clock · lane · loss · storage policy
   │                                 └─ profiling_abi: vocabulary + REGISTRY + LANES + ARM_MASK + macros
   ▲            ▲              ▲              ▲
   │            │              │              │
boyko_rhi   boyko_threadpool   boyko_ecs      boyko_log ── (sinks, records; never names ZoneId)
 _vulkan                        └─ ecs::core::profiling: store, fold, contrast, ECS control
```

Acyclicity: `boyko_diag` has out-degree 0, so no crate that depends on it is reachable from it. The full 21-manifest edge list and the general form of that proof are `substrate/crate-graph`'s; what this plan owns are the two edges it adds, both **downward, in-house, zero third-party** (rev 3's two `→ boyko_utils` edges are **withdrawn** and replaced):

| Edge | Why | Legality |
|---|---|---|
| `boyko_rhi_vulkan` → `boyko_diag` | so `gpu_zone.rs` and the `vkCmd*` census sites can use `declare_zone!` / `counter!` at the site that owns the command | Precedent in the same file: `boyko_rhi_vulkan/Cargo.toml:44-49` admits `boyko_sdf_math` as *"a `no_std`, graphics-free leaf with ZERO third-party deps — does not breach the 'no ash/vulkano/windows-sys/libc' constraint above"*. `boyko_diag` is the same shape: std only, zero deps, no `ash`/`vulkano`/`windows-sys`/`libc`. A `boyko_diag` row is added to that in-file rationale block |
| `boyko_threadpool` → `boyko_diag` | so `worker_main` / `ThreadPool::install` can set the shared lane TLS once per thread (F12 / S3) | `boyko_diag` has zero dependencies, so no cycle is possible. `crates/boyko_threadpool/Cargo.toml` today lists `crossbeam-deque` / `crossbeam-utils` only (plus `loom` under `cfg(loom)`) |

`boyko_ecs` additionally gains `→ boyko_diag` and `→ boyko_log` (the fold is what *emits* every `W92xx`). Those two are the logging plan's edges as much as this one's and are listed in both.

**Rejected alternative (and why it is worse).** Keep the vocabulary in `boyko_ecs` and hand the recorder an *opaque* `u16` label, binding label→`ZoneId` in `boyko_render`. It needs no Cargo change — and it **reintroduces `VbTimedPass`'s hand-maintained table** (`crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:311-329`), the exact property `declare_zone!`'s required `name =` was introduced to buy (D6). The name and the `vkCmdWriteTimestamp` that carries it would live in different crates, which is D13 rule 1 ("counts originate AT the operation they count") violated for the label.

**Rejected alternative 2: put `VmReservation` in `boyko_diag` so the leaf can own its own memory.** `VmReservation` is `pub(crate)` in `boyko_ecs` (`crates/boyko_ecs/src/ecs/memory/vm.rs:85`) and its unix arm calls `libc` (`vm.rs:149`). Moving it down needs either a third-party dep in the zero-dep leaf (forbidden) or a **second** hand-declared per-OS backing, against `vm.rs:12-17`'s single-source-of-truth clause. Inventing memory backing twice is a worse Principle-0 breach than the one it would fix — hence S12's extent rule, owned by `substrate/never-freed-storage`.

**What did NOT move.** The `Profiler` **Resource**, the frame-major columns, the fold, the statistics, `Floor`/`Twin`/`resolve`, the concurrency analysis and the ECS control surface all stay in `boyko_ecs`. `boyko_diag` gets no `Resource`, no `World`, no allocator, no thread, no file, no print — **the leaf is diagnostically mute**: every condition it observes is a sticky `DiagFlag` + a counter, read and emitted by `boyko_ecs`'s fold (`substrate/mute-leaf-rule`). Principle 0 is satisfied where it applies: **the durable store is an ECS `Resource`, on kernel VM-native storage**.

### Principle 0, honestly (F7)

Rev 2 justified the transport rings by analogy to `EventBuffer`'s lanes, *"Principle 0's own named exception"*. **That analogy is refuted, and rev 2 refuted it itself one page later.** `crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:202` — `pub(crate) lanes: Box<[ThreadLanePair<E>]>` is a **field of `EventBuffer`**, owned by `EventDispatcher`, reachable through the world. It is neither a `static` nor "threadpool internals". Rev 2's D15 says so explicitly: *"a `static LANES` has no `&mut` to stand in for that clause."* Citing it for Principle 0 after refuting it for `Sync` was internally inconsistent. **The analogy is withdrawn.**

The argument is made on its own terms instead:

1. **The emitters cannot reach a world.** They are (a) the executor, running outside any system's param set; (b) worker closures holding only a raw `SystemBox` pointer; (c) the host thread outside `ThreadPool::install`, which has no world; (d) `boyko_rhi_vulkan` recorder code, which cannot name `EcsMaster` at all (F1's graph). Reaching a `Resource` from those needs a published `NonNull`, a null check on the hot path, and a world-drop lifetime hazard — to arrive at **the same bytes**.
2. **The category is the kernel's own storage implementation** — Principle 0's first named exception — not a "parallel data system". Nothing durable and nothing queryable lives in the rings; a `Sample` is alive for at most one frame before the fold folds it into the `Resource`.
3. **The backing memory is the kernel's, not `std`'s.** Rev 2 put 6.7 MiB on `Box<[T]>`. The standing owner correction is *"VM-native storage, NOT `std::Vec` — even inside a `Resource`."* So: **`boyko_diag::profiling_abi` allocates nothing.** `LANES` holds `AtomicPtr<Sample>` control blocks in `.bss`; `arm()` — which runs in `boyko_ecs` and has a world — reserves and commits one `VmReservation` (`vm.rs:109`), publishes each lane's base pointer `Release`, and keeps the reservation in the `Profiler` `Resource`. The store columns are **offsets into** that reservation, reached through accessors over a `NonNull` base — never `&'static mut` slices (B4, D15). The `Box<[T]>` in rev 2's `Profiler` is deleted.
4. **Where the boundary between `.bss` and `VmReservation` falls, stated as a rule rather than a plea (S12).** *Extent known at compile time ⇒ `.bss` static. Extent chosen at run time from config ⇒ `VmReservation`, and the owner must therefore sit at or above `boyko_ecs`.* `.bss` is not what the owner's standing correction targets: it is demand-zero, address-stable and allocation-free — exactly like a reservation — and the *only* property separating the two is whether the extent is a run-time quantity. Applied here: lane control blocks, `REGISTRY`, `DYN_DESCS`/`DYN_NAMES`, the folded `ZoneHandle` statics and the telemetry double buffer are `.bss`; the sample slab and every store column, whose extent comes from `ProfilerConfig` at `arm`, are the reservation. The boundary is forced anyway by the rejected alternative above. Both halves are measured by the same gate in three domains (G23) — including the `.bss` domain, which rev 3's two-domain gate **structurally could not see** (M10). The rule itself is owned by `substrate/never-freed-storage`; this is its application.

### Invariants that must survive

1. **Disarmed ⇒ byte-identical recorded command stream.** Enforced by a **command census**, not by image hashes (D17 / G5). `goldens/PINS.toml:3` pins the SHA-256 of a dumped BMP and is structurally blind to commands that draw no pixels.
2. **`SystemMeta` is 256 B.** Existing pin: the unit test at `crates/boyko_ecs/src/ecs/core/system/system_meta.rs:421`. Field bytes 232 + 8 (`gpu_intent`, `:128`) + 1 (`requires_dispatcher`, `:141`) = 241; the new `zone: ZoneId` (`u16`) lands at offset 242 → 244 ≤ 256; align 32 is unchanged (`:429` pins it). The field is **unconditional in both the feature axis and the tier axis**, so the pin is configuration-independent. A `const _: () = assert!(size_of::<SystemMeta>() == 256)` is added beside the test.
3. **`Schedule::systems` element address stability** — the executor mints raw pointers per dispatch. No field is added to `SystemBox`; no reference is taken across the spawn boundary.
4. **VB-P1d's published numbers keep their meaning** — slots 0/1/2 were defined against `TOP_OF_PIPE` begins; their successor zones declare `GpuStage::TopOfPipe` and therefore can never join a partition group (D7).
5. **`timestampValidBits` masking before subtraction** stays at the RHI seam (`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:1249`, masking at `:1257-1265`).
6. **Ticks, not ns, at the seam** (`crates/boyko_rhi/src/device.rs:891-903`): recovering ticks by dividing an `f64` back through `timestampPeriod` launders the measurement through the factor under characterisation. The seam's own doc says it — *"`timestampPeriod` is the ns-per-tick SCALE, not the STEP"*.
7. **Principle 0, with S12's extent rule.** The durable store is a `Resource` on `VmReservation` because its extent is a run-time quantity (`ProfilerConfig`); the transport control blocks, the registry and the dynamic arenas are `.bss` because their extents are `const`. The transport is kernel-internal, allocation-free, and allocated *by* the kernel. **No `&'static mut` anywhere**: a slice of that type derived from memory the same struct owns is a self-referential aliasing violation that Tree Borrows flags, so the columns follow the kernel's own precedent — a `NonNull` base plus accessors, as `VmColumn` (`crates/boyko_ecs/src/ecs/memory/vm_column.rs:70,88`) and `ComponentPool` do (B4).
8. **One process-global panic hook, owned by the logging plan — and the profiler's callback is registered, not installed** (X24 / S5). The invariant survives unchanged; its text now lives once, in `SEAM.md` under `seam/lifecycle-order`, because it is a joint object: `boyko_log` owns the `PRE_FLUSH` table, `Profiler::arm()` registers into it, and the teardown order (`flush_gpu()` ahead of the log flush) is a property of the host, not of either subsystem. `profiling/02-GPU.md` and `profiling/04-GAME-FACING.md` are the files that consume it.

### Hard environmental constraints (all re-verified this revision)

- **`VK_PRESENT_MODE_FIFO_KHR` is unconditional** (`crates/boyko_rhi_vulkan/src/present/swapchain.rs:199`); `present_mode_supported` is defined at `crates/boyko_rhi_vulkan/src/present/surface.rs:218`, imported at `present/swapchain.rs:10` and already used at `present/swapchain.rs:164`. Any end-to-end wall clock is bounded below by the refresh interval.
- **The decidability floor is not a constant**: 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol (`docs/VG-DECIDABILITY-FLOOR.md`). The tree's definition (`crates/boyko_app/tests/vg_decidability_floor.rs:27-73`) is `FLOOR_SIGMA(3.0, :73) × CV of the WORST statistic of the SHIPPED bench class`, over `DEFAULT_SESSIONS = 7` (`:59`) **separate processes**, repeated `DEFAULT_REPEATS = 3` (`:68`), with the stated reason at `:28-30`: *"a floor established on a different instrument bounds nothing about this one."*
- **`Instant::now()` costs ~20-30 ns per PAIR** — `crates/bench_bevy_vs_boyko/benches/profile_spawn.rs:229-230`: *"each pair of `now()` calls costs ~20-30 ns."* Rev 2 said "≈ 25 ns/call, ≈ 60 ns/pair", which is 2× inflated with the units inverted, and the error's direction favoured the design being chosen (F21). **Corrected.** The rejection survives on the corrected number — 20-30 ns for the *clock alone* is 2× the ≤ 12 ns budget for the whole open+close — but it is no longer a 5× argument, and the doc says so.
- **No engine crate spawns threads.** `thread::spawn` / `thread::Builder` appear only in `boyko_threadpool/{sync,thread_pool}.rs` and two non-render sites. `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:442-445` states it: *"Recorder and readback are the same thread today (the runner drives `record_vb` and the post-present readback in one loop)."* **Rev 4 adds no thread.**
- **There is NO non-blocking fence-status verb.** `frame_driver.rs:265` is `submission_epoch()`, a getter for a **submit counter** that advances at `vkQueueSubmit` — rev 2 called it "the fence gate … which is signalled", which it is not (F13). Every fence operation in the tree is `wait_for_fences(..., VK_TIMEOUT_INFINITE)` (`frame_driver.rs:319`, `:400`). **But the counter's own doc supplies the derivation** (`frame_driver.rs:255-262`): *"a resource enqueued at epoch `N` is safe to free once the host has observed epoch `>= N + FRAMES_IN_FLIGHT` (its last possible submit, `N`, is then guaranteed GPU-complete by the ring's own fence discipline)."* That is the asset-retire rule, it is non-blocking, and it is already an ECS `Resource`: `boyko_render::asset_refcount::RenderEpoch` (`crates/boyko_render/src/asset_refcount.rs:55`), written by the host at `crates/boyko_app/src/runner.rs:1320`. **`fence_seen` is derived from it. No new RHI fence verb is needed** (D4a).
- **`WORKER_ID_DISPATCHER` is set only inside `ThreadPool::install`**; outside it a thread is `WORKER_ID_UNATTACHED` (`crates/boyko_threadpool/src/tls.rs:24,29`), which `current_worker_id_or_dispatcher_lane` maps to lane **0** = worker 0's lane (`tls.rs:69-78`). The profiler must not reuse that mapping (D2).
- **`EnableTag` toggles fire NO hook and NO observer.** `crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:77-88` — *"O(1) warm: no migration, no structural-generation bump, **no hook / observer fire**, no deferred drain."* This **refutes the scope extension's proposed mechanism** (an observer on the `IsEnabled` transition projecting into `ARM_MASK`). See D20 (`profiling/04-GAME-FACING.md`) for the replacement. `is_enabled::<T>` is documented at `:100-105` as *"O(1) … ≤ 5 ns"*, which is what makes the replacement cheap.
- **An enable-bit tag must be a FIELDLESS struct, and the toggle needs `&mut EcsMaster`** (B2, both re-verified this revision). `crates/boyko_macros/src/component.rs:580-604` — `reject_non_zst_bitset_tag` accepts only `Fields::Unit` or an empty named/tuple struct and emits *"`#[component(storage = "bitset")]` requires a fieldless struct … a bitset enable tag has no ComponentPool, so any field data would have nowhere to live"*; enums and unions are rejected too. `EcsMaster::enable::<T>` / `disable::<T>` take **`&mut self`** (`enable_tag_api.rs:87`, `:95`), which no parallel system can hold. **A correction to the review, recorded because it makes the defect worse, not better:** the review placed the storage-kind `debug_assert_eq!` on `is_enabled`; in the tree it is on the *write* path (`set_enable_bit`, `:148-155`). The *read* path `is_enabled → test_enable_bit` (`:201-215`) has **no assert at all** — it looks up `archetype.enable_store.column(tag)`, gets `None` for a non-bitset id and returns `false`. So rev 3's fielded `ProfilingScope` would not have panicked in debug; it would have projected an **all-zero mask in every build, silently**, i.e. a profiler permanently disarmed with no diagnostic. D20 carries the fix.
- **A deferred enable/disable exists and lands inside the same frame.** `EntityCommands::enable::<T>()` / `disable::<T>()` (`crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs:220`, `:236`, plus `_id` variants at `:249`, `:262`) push an `EnableTagCommand` — *"a single POD payload (an `Entity`, an `EnableTagId`, and a `bool`)"* (`commands/enable_tag_commands.rs:1-20`) — and the executor applies each system's queue under the exclusive `&mut EcsMaster` it already holds, **at that system's completion inside the schedule run** (`schedule.rs:722-726` concurrent path, `:1130-1133` dispatcher-inline path). This is what gives D20 a write path callable from an ordinary parallel system with **no exclusive system and no schedule serialisation point**, at exactly the one-frame latency G12 already asserts.
- **`MAX_WORKERS = 64`** (`crates/boyko_threadpool/src/thread_pool.rs:49`), and the requested worker count is clamped to `[1, MAX_WORKERS]` (`:554`). The shared lane registry's 64 worker slots are that constant, not a guess (S3, `substrate/lane-registry`).
- **`MAX_SYSTEMS_PER_SCHEDULE = 1024`** (`crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:70`). Any profiler-side system bound must be that number or must count what it truncates (F5).
- **`VmReservation`'s real surface** (M10 — rev 3 cited a method that does not exist). `crates/boyko_ecs/src/ecs/memory/vm.rs:85` — `pub(crate) struct VmReservation { base: NonNull<u8>, os_len: usize, … }`, accessors `base() -> NonNull<u8>` (`:184`) and **`os_len() -> usize`** (`:190`); `reserve` at `:109`; `commit` at `:199`; **`impl Drop`** at `:263` (it unmaps). There is **no `reserved_bytes()`**. It is `pub(crate)`, so no gate outside `boyko_ecs` can call it — G23 reads `Profiler::reserved_bytes()`, a public accessor over `vm.os_len()`, instead of widening the kernel type's visibility. It is documented `!Send`/`!Sync` (`vm.rs:82-84`: *"owners that are shared across threads opt in with their own `unsafe impl` and their own exclusivity argument"*), while `pub trait Resource: 'static + Send + Sync + Sized` (`crates/boyko_ecs/src/ecs/core/resources/resource.rs:42`) — so a `Profiler` holding one owes an explicit `unsafe impl Send + Sync` with an argument, and that obligation is now in the unsafe inventory and on the Miri list (B4/D15).
- **On minimise the ECS frame keeps running while submits stop** (M13). `crates/boyko_app/src/runner.rs:1320` publishes `RenderEpoch` from `submission_epoch()` and `:1321` runs `app.update_with_delta(dt)`; the 0×0-client check at `:1328-1332` then `continue`s **before** `wait_frame_in_flight` (`:1336`), the record and the submit. So folds, `Res<Profiler>` readers and telemetry all keep running while `RenderEpoch` is frozen — an epoch-only retire deadline can never fire, and teardown is never reached because the process is alive. (Rev 3 also cited `:1319` for the publication; the line is `:1320`.)
- A test/bench binary whose name contains `time` / `update` / `setup` / `install` / `patch` triggers Windows UAC os-error-740.
- `cargo test --workspace --lib` does not build `tests/`; root `cargo check --all-targets` is vacuous (virtual-manifest quirk). Gates use `--workspace` and name the target.
- `debug_assert!` protects nothing in the GPU timing path — it inherits the driver's release profile. `crates/boyko_app/src/gpu_scene/mod.rs:7498` states the limit of the tree's own equality check in the same words: *"this is an implementation-equality check that runs in the DEV profile. A release bench run (the timing worker inherits the driver's profile) does not execute it."*
- `crates/boyko_rhi_vulkan/src/ffi.rs:846,849` declares `VK_QUERY_RESULT_64_BIT = 0x1` and `WAIT_BIT = 0x2`. **`WITH_AVAILABILITY_BIT` is `0x0000_0004`** and is not declared (0 occurrences tree-wide). `hostQueryReset` exists as a feature field (`ffi.rs:2716`), is never enabled, and `vkResetQueryPool` is not loaded.
- **This repo has no kill-after-timeout test pattern** (`crates/boyko_app/tests/vb_bench_totality_gate.rs:44-53`, whose own doc names it: *"this repository has no kill-after-timeout pattern to borrow"*), so a red that manifests as a hung CI job is **not a showable red**. Every hang-freedom property must be proved structurally or at compile time.

**Where these constraints bind, and who owns the decision they force.** The FIFO bullet and the `ffi.rs` / `hostQueryReset` bullet force D12 and D18 (`profiling/02-GPU.md`); the three `EnableTag` bullets force D20 (`profiling/04-GAME-FACING.md`); the decidability-floor and lattice bullets force the statistics discipline (`profiling/03-STATISTICS.md`). Those files cite the decision; they do not restate the bullet, so there is exactly one place where a re-verification lands.

---

## Measured facts this file does not own

The seven measured facts from VG R3 P4-6 (`cf2d367`) — the statistics discipline this design inherits, including the `WINDOW = 121` consequence and the in-tree 16-vs-32 ns lattice doc-rot — are owned by `profiling/03-STATISTICS.md` (`profiling/statistics-discipline`). They are the reason several numbers in this file are stated as bands rather than points, and they are cited from here rather than duplicated.
