# Architecture: Profiling System (`boyko_utils::profiling_abi` + `boyko_ecs::ecs::core::profiling` + RHI zone seam) — rev 3

**Status:** design, pre-implementation. **Target file:** `docs/PROFILING-SYSTEM-PLAN.md`.
**Revision:** rev 3. Folds **two** inputs into one design:

1. the second-pass architecture review of rev 2 — verdict **REJECTED**, 8 BLOCKER / 13 MAJOR / 7 MINOR (`F1`…`F28`);
2. an owner-stated **scope extension**: the profiler is used not only to evaluate the engine but **by the games themselves** — collect as much data as possible, be maximally flexible (`X1`…`X25`).

Both disposition tables are at the end and are the changelog. **Silence on a finding is itself a defect**, so every `F` and every `X` has a row, including the ones this revision *refutes*.

**Supersedes:** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` (all three collectors), the `BOYKO_VB_BENCH` / `BOYKO_SV0_BENCH` runner harness bodies, and the private statistics helpers in `crates/boyko_app/src/runner.rs`.

**Module paths, stated once** (rev 2 gave three, all wrong — F24):

| Half | Path | Crate |
|---|---|---|
| Emission ABI: vocabulary, registry, transport, macros, clock | `boyko_utils::profiling_abi` | `boyko-utils` (zero-dependency leaf) |
| Store, statistics, contrast, ECS control surface | `boyko_ecs::ecs::core::profiling` | `boyko-ecs` |
| GPU recorder | `boyko_rhi_vulkan::present::gpu_zone` | `boyko_rhi_vulkan` |
| Host retire + reporter + artifact + stream | `boyko_render::profiling_bridge`, `boyko_app::profiling::{report,artifact,stream}` | — |

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

### Where the two audiences conflict — named, decided, and costed

A game-facing profiler and an engine-measurement profiler want opposite things in six places. Each is decided here, with the cost to the side that loses.

| # | Conflict | Decision | Cost to the losing side |
|---|---|---|---|
| **C-I** | Fixed-capacity lossy rings (what buys 12 ns emission) vs "collect as much as possible" over an hours-long session | Ring stays fixed and lossy. Hours are served by **lifetime accumulators** (tier B) + **log-linear histograms** (tier C) + a **window-granular binary stream** (D23) | **Game:** no per-frame history beyond `WINDOW` frames, ever. An hour of per-frame rows must be reduced offline from the stream |
| **C-II** | Static `declare_zone!` handles (0-instruction disabled path, tier-foldable) vs zones defined by data / config / script / mods | Both, in a **partitioned** id space and a **partitioned** ring region (D19) | **Game:** a dynamic zone costs ≤ 14 ns (≤ 18 ns across FFI) instead of ≤ 12, cannot be compile-time tier-folded, and is refused past a budget declared at `arm` |
| **C-III** | Terminal exhaustion (`E9201`, a loud engine diagnostic) vs a shipping title that must never panic on a diagnostic | **Non-terminal everywhere.** Exhaustion yields `ZoneId::DISABLED`, a counted refusal and a once-per-session warning | **Engine:** a mis-sized budget now shows up as a `zones_refused` field instead of a crash. G11 is the gate that makes the field non-vacuous. This is a deliberate reversal of rev 2 (F5) |
| **C-IV** | "The instrument is outside its own primary number" (D16) vs a game reading its own counters *while the frame runs* | Reads are **windowed and lagged**: CPU data is frame N−1, GPU is N−4…N−2 (D25) | **Game:** no same-frame readback. LOD / dynamic-resolution decisions consume N−1. Same-frame counter readback as a message bus is **refused** (X14) |
| **C-V** | Contrast strictness (a bench must refuse a truncated window) vs a game whose rings drop routinely | `resolve` refuses any leg whose window carried a drop or crossed a clock epoch: `NotResolved { reason }` | **Engine bench:** a bench that drops now produces *no number* instead of a wrong one. This makes the engine side **stricter**, not looser |
| **C-VI** | `profiling` default-on so shipped code carries the sites vs "zero overhead in a shipping game" | Feature default-on; an **orthogonal compile-time tier axis** (`Always` / `Dev` / `Deep`) folds sites out of a retail build (D21) | **Engine:** changing the tier rebuilds the workspace, and ~12 KiB of dead `.bss` remains for folded handles |

### Performance targets — three configurations, all **measured**, none asserted

| Metric | Dev, armed | Dev, disarmed | Retail (`Always` tier) | Proved by |
|---|---|---|---|---|
| static zone open+close | ≤ 12 ns median | ≤ 2 ns | ≤ 2 ns | criterion `zone_cost`, six legs one sitting; regression gate at +25 % |
| dynamic zone open+close | ≤ 14 ns | ≤ 3 ns | n/a (`Deep`-folded) | `zone_cost` dyn legs (G17) |
| zone at a tier above `GLOBAL_TIER` | **0 instructions** | 0 | 0 | **two-sided token-level expansion test + object-symbol census** (G1, G14) — *not* the rev-2 "the macro cannot name the recorder" argument, which proved a different proposition (F8) |
| drain, armed | ≤ 5 µs/frame at 400 samples, `zone_stride ≤ 1280` | 1 load + 1 branch | ≤ 2 µs (no analysis columns) | criterion `drain_cost`, 4 legs; `__drain` is itself a zone |
| allocations/frame while armed | 0 | 0 | 0 | zero-alloc gate (existing 19-test pattern) |
| profiling `vkCmd*` recorded while disarmed | **0** | **0** | **0** | command census, two-sided **with an equality on the armed side** (G5) |
| resident, armed (`Z = 1024`) | ≤ 7 MiB (computed 6.65) | ≤ 7 MiB after first arm (D15) | **≤ 1 MiB** (computed 0.85) | boot-allocation gate over **both** the counting allocator and `VmReservation::reserved_bytes()`, two-sided, plus "a second `arm` allocates 0" (G23) |
| resident, armed, `profiling-analysis` on | ≤ 7.5 MiB (computed 7.0) | — | n/a | same gate |
| resident, `dyn_budget = 3072` | ≤ 21 MiB, **and `W9211` fires** | — | n/a | same gate; the ceiling is the *declared* budget's, not a promise the game will not ask for it |
| GPU readback blocking | **never** | never | never | `VK_QUERY_RESULT_WAIT_BIT` is **unrepresentable** in the new verb — a `const _` assert, i.e. a *compile* error (G2a) |
| telemetry write | p95 ≤ 200 µs per 2 s window | — | same | criterion `stream_encode` |

---

## Context and constraints

### The crate graph, and why the GPU half is buildable at all (F1)

Rev 2 declared the GPU zone vocabulary in `boyko_ecs` and used it inside `boyko_rhi_vulkan`. **That does not compile.** Verified against HEAD:

- `crates/boyko_rhi_vulkan/Cargo.toml:42-49` — dependencies are `boyko_rhi` and `boyko_sdf_math` only (plus `windows-sys` under `cfg(windows)`).
- `crates/boyko_rhi/Cargo.toml:7-10` — dependency is `boyko-utils` only.
- Neither can name `ZoneId`, `GpuStage`, `PartitionGroup`, `declare_zone!` or `counter!` if those live in `boyko_ecs`.
- The tree states the rule against the naive fix (add `boyko-ecs` to the backend): `crates/boyko_render/Cargo.toml:44-50` — *"the low-level `boyko_rhi_vulkan` backend must not depend upward on the scene crate."*

**Decision: the emission ABI moves DOWN to the one crate that is already below everything — `boyko-utils`.**

`crates/boyko_utils/Cargo.toml` has an **empty `[dependencies]`** section; `crates/boyko_utils/src/lib.rs` is four modules (`sparse_map`, `identifiers`, `bit_mask`, `type_intern`). `type_intern` is the precedent, exactly: the 2026-07 hot-path audit found five production sites that needed one process-wide intern registry across crate boundaries and the answer was to put the kernel primitive in the leaf. **A zone registry is the same category of object.**

```
boyko_utils  (zero deps)  ── profiling_abi: vocabulary + REGISTRY + LANES + ARM_MASK + macros + clock
   ▲            ▲              ▲
   │            │              │
boyko_rhi   boyko_threadpool   boyko_ecs ── ecs::core::profiling: store, drain, contrast, ECS control
   ▲            ▲                 ▲
boyko_rhi_vulkan ────────────────┘ (via boyko_render / boyko_app)
```

Two Cargo edges are added, both **downward, in-house, zero third-party**:

| Edge | Why | Legality |
|---|---|---|
| `boyko_rhi_vulkan` → `boyko-utils` | so `gpu_zone.rs` and the `vkCmd*` census sites can use `declare_zone!` / `counter!` at the site that owns the command | Precedent in the same file: `boyko_rhi_vulkan/Cargo.toml:44-49` admits `boyko_sdf_math` as *"a `no_std`, graphics-free leaf with ZERO third-party deps — does not breach the 'no ash/vulkano/windows-sys/libc' constraint above"*. `boyko-utils` is the same shape and has strictly fewer dependencies (none) |
| `boyko-threadpool` → `boyko-utils` | so `worker_main` / `ThreadPool::install` can set the ABI's `PROFILER_LANE` TLS once per thread (F12) | `boyko-utils` has zero dependencies, so no cycle is possible. `boyko-threadpool` today depends only on `crossbeam-deque` / `crossbeam-utils` |

**Rejected alternative (and why it is worse).** Keep the vocabulary in `boyko_ecs` and hand the recorder an *opaque* `u16` label, binding label→`ZoneId` in `boyko_render`. It needs no Cargo change — and it **reintroduces `VbTimedPass`'s hand-maintained table**, the exact property `declare_zone!`'s required `name =` was introduced to buy (D6). The name and the `vkCmdWriteTimestamp` that carries it would live in different crates, which is D13 rule 1 ("counts originate AT the operation they count") violated for the label.

**What did NOT move.** The `Profiler` **Resource**, the frame-major columns, the drain, the statistics, `Floor`/`Twin`/`resolve`, the concurrency analysis and the ECS control surface all stay in `boyko_ecs`. `boyko_utils` gets no `Resource`, no `World`, no allocation (see the next paragraph). Principle 0 is satisfied where it applies: **the durable store is an ECS `Resource`, on kernel VM-native storage**.

### Principle 0, honestly (F7)

Rev 2 justified the transport rings by analogy to `EventBuffer`'s lanes, *"Principle 0's own named exception"*. **That analogy is refuted, and rev 2 refuted it itself one page later.** `crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:202` — `pub(crate) lanes: Box<[ThreadLanePair<E>]>` is a **field of `EventBuffer`**, owned by `EventDispatcher`, reachable through the world. It is neither a `static` nor "threadpool internals". Rev 2's D15 says so explicitly: *"a `static LANES` has no `&mut` to stand in for that clause."* Citing it for Principle 0 after refuting it for `Sync` was internally inconsistent. **The analogy is withdrawn.**

The argument is made on its own terms instead:

1. **The emitters cannot reach a world.** They are (a) the executor, running outside any system's param set; (b) worker closures holding only a raw `SystemBox` pointer; (c) the host thread outside `ThreadPool::install`, which has no world; (d) `boyko_rhi_vulkan` recorder code, which cannot name `EcsMaster` at all (F1's graph). Reaching a `Resource` from those needs a published `NonNull`, a null check on the hot path, and a world-drop lifetime hazard — to arrive at **the same bytes**.
2. **The category is the kernel's own storage implementation** — Principle 0's first named exception — not a "parallel data system". Nothing durable and nothing queryable lives in the rings; a `Sample` is alive for at most one frame before the drain folds it into the `Resource`.
3. **The backing memory is the kernel's, not `std`'s.** Rev 2 put 6.7 MiB on `Box<[T]>`. The standing owner correction is *"VM-native storage, NOT `std::Vec` — even inside a `Resource`."* So: **`boyko_utils::profiling_abi` allocates nothing.** `LANES` holds `AtomicPtr<Sample>` control blocks in `.bss`; `arm()` — which runs in `boyko_ecs` and has a world — reserves and commits one `VmReservation` (`crates/boyko_ecs/src/ecs/memory/vm.rs:99`), publishes each lane's base pointer `Release`, and keeps the reservation in the `Profiler` `Resource`. The store columns are slices of the same reservation. The `Box<[T]>` in rev 2's `Profiler` is deleted.

### Invariants that must survive

1. **Disarmed ⇒ byte-identical recorded command stream.** Enforced by a **command census**, not by image hashes (D17 / G5). `goldens/PINS.toml:3` pins the SHA-256 of a dumped BMP and is structurally blind to commands that draw no pixels.
2. **`SystemMeta` is 256 B.** Existing pin: the unit test at `crates/boyko_ecs/src/ecs/core/system/system_meta.rs:421`. Field bytes 232 + 8 (`gpu_intent`, `:128`) + 1 (`requires_dispatcher`, `:141`) = 241; the new `zone: ZoneId` (`u16`) lands at offset 242 → 244 ≤ 256; align 32 is unchanged (`:429` pins it). The field is **unconditional in both the feature axis and the tier axis**, so the pin is configuration-independent. A `const _: () = assert!(size_of::<SystemMeta>() == 256)` is added beside the test.
3. **`Schedule::systems` element address stability** — the executor mints raw pointers per dispatch. No field is added to `SystemBox`; no reference is taken across the spawn boundary.
4. **VB-P1d's published numbers keep their meaning** — slots 0/1/2 were defined against `TOP_OF_PIPE` begins; their successor zones declare `GpuStage::TopOfPipe` and therefore can never join a partition group (D7).
5. **`timestampValidBits` masking before subtraction** stays at the RHI seam (`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:1249`, masking at `:1257-1265`).
6. **Ticks, not ns, at the seam** (`crates/boyko_rhi/src/device.rs:891-903`): recovering ticks by dividing an `f64` back through `timestampPeriod` launders the measurement through the factor under characterisation. The seam's own doc says it.
7. **Principle 0** — see the section above. The durable store is a `Resource` on `VmReservation`; the transport is kernel-internal, allocation-free, and allocated *by* the kernel.
8. **One process-global panic hook, owned by the logging plan.** The profiler exposes `#[cold] flush_on_panic()` for it to call and installs nothing (X24).

### Hard environmental constraints (all re-verified this revision)

- **`VK_PRESENT_MODE_FIFO_KHR` is unconditional** (`crates/boyko_rhi_vulkan/src/present/swapchain.rs:199`); `present_mode_supported` is defined at `crates/boyko_rhi_vulkan/src/present/surface.rs:218`, imported at `present/swapchain.rs:10` and already used at `present/swapchain.rs:164`. Any end-to-end wall clock is bounded below by the refresh interval.
- **The decidability floor is not a constant**: 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol (`docs/VG-DECIDABILITY-FLOOR.md`). The tree's definition (`crates/boyko_app/tests/vg_decidability_floor.rs:27-73`) is `FLOOR_SIGMA(3.0, :73) × CV of the WORST statistic of the SHIPPED bench class`, over `DEFAULT_SESSIONS = 7` (`:59`) **separate processes**, repeated `DEFAULT_REPEATS = 3` (`:68`), with the stated reason at `:28-30`: *"a floor established on a different instrument bounds nothing about this one."*
- **`Instant::now()` costs ~20-30 ns per PAIR** — `crates/bench_bevy_vs_boyko/benches/profile_spawn.rs:229-230`: *"each pair of `now()` calls costs ~20-30 ns."* Rev 2 said "≈ 25 ns/call, ≈ 60 ns/pair", which is 2× inflated with the units inverted, and the error's direction favoured the design being chosen (F21). **Corrected.** The rejection survives on the corrected number — 20-30 ns for the *clock alone* is 2× the ≤ 12 ns budget for the whole open+close — but it is no longer a 5× argument, and the doc says so.
- **No engine crate spawns threads.** `thread::spawn` / `thread::Builder` appear only in `boyko_threadpool/{sync,thread_pool}.rs` and two non-render sites. `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:442-445` states it: *"Recorder and readback are the same thread today (the runner drives `record_vb` and the post-present readback in one loop)."* **Rev 3 adds no thread.**
- **There is NO non-blocking fence-status verb.** `frame_driver.rs:265` is `submission_epoch()`, a getter for a **submit counter** that advances at `vkQueueSubmit` — rev 2 called it "the fence gate … which is signalled", which it is not (F13). Every fence operation in the tree is `wait_for_fences(..., VK_TIMEOUT_INFINITE)` (`frame_driver.rs:319`, `:400`). **But the counter's own doc supplies the derivation** (`frame_driver.rs:255-262`): *"a resource enqueued at epoch `N` is safe to free once the host has observed epoch `>= N + FRAMES_IN_FLIGHT` (its last possible submit, `N`, is then guaranteed GPU-complete by the ring's own fence discipline)."* That is the asset-retire rule, it is non-blocking, and it is already an ECS `Resource`: `boyko_render::asset_refcount::RenderEpoch` (`crates/boyko_render/src/asset_refcount.rs:55`), written by the host at `crates/boyko_app/src/runner.rs:1319`. **`fence_seen` is derived from it. No new RHI fence verb is needed** (D4a).
- **`WORKER_ID_DISPATCHER` is set only inside `ThreadPool::install`**; outside it a thread is `WORKER_ID_UNATTACHED` (`crates/boyko_threadpool/src/tls.rs:24,29`), which `current_worker_id_or_dispatcher_lane` maps to lane **0** = worker 0's lane (`tls.rs:69-78`). The profiler must not reuse that mapping (D2).
- **`EnableTag` toggles fire NO hook and NO observer.** `crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:77-88` — *"O(1) warm: no migration, no structural-generation bump, **no hook / observer fire**, no deferred drain."* This **refutes the scope extension's proposed mechanism** (an observer on the `IsEnabled` transition projecting into `ARM_MASK`). See D20 for the replacement. `is_enabled::<T>` is documented at `:100-105` as *"O(1) … ≤ 5 ns"*, which is what makes the replacement cheap.
- **`MAX_SYSTEMS_PER_SCHEDULE = 1024`** (`crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:70`). Any profiler-side system bound must be that number or must count what it truncates (F5).
- A test/bench binary whose name contains `time` / `update` / `setup` / `install` / `patch` triggers Windows UAC os-error-740.
- `cargo test --workspace --lib` does not build `tests/`; root `cargo check --all-targets` is vacuous (virtual-manifest quirk). Gates use `--workspace` and name the target.
- `debug_assert!` protects nothing in the GPU timing path — it inherits the driver's release profile (`crates/boyko_app/src/gpu_scene/mod.rs:7498`).
- `crates/boyko_rhi_vulkan/src/ffi.rs:846,849` declares `VK_QUERY_RESULT_64_BIT = 0x1` and `WAIT_BIT = 0x2`. **`WITH_AVAILABILITY_BIT` is `0x0000_0004`** and is not declared (0 occurrences tree-wide). `hostQueryReset` exists as a feature field (`ffi.rs:2716`), is never enabled, and `vkResetQueryPool` is not loaded.
- **This repo has no kill-after-timeout test pattern** (`crates/boyko_app/tests/vb_bench_totality_gate.rs:44-53`), so a red that manifests as a hung CI job is **not a showable red**. Every hang-freedom property must be proved structurally or at compile time.

### Measured facts from VG R3 P4-6 (`cf2d367`) — the statistics discipline this design inherits

P4-6 needed four sittings; **every failure was a defect in the measurement, not in the engine**. The plan's statistics are built on these, because a game-facing profiler will hit them constantly. Full treatment in §Statistics discipline.

1. **Two adjacent `BOTTOM_OF_PIPE` stamps cannot establish a strict order** — they resolve on the same tick. This kills rev 2's `__gpu_null` "quantum probe" (F6): its measured value on this box is **0**, every time.
2. **Equal timestamps cannot license a conclusion about RECORD ORDER.** Record order is a host property and must be witnessed host-side.
3. **`median(off) + median(dur) ≠ median(off + dur)`** — composing medians crossed a true inequality by 144-240 ns.
4. **A zero twin whose expected value is exactly zero measures DRIFT, not RESOLUTION.** A0/A1 were the same configuration on a serialized deterministic GPU; the twin came back exactly 0 on all ten passes, and the verdict rule silently collapsed from "clears the noise" to "is nonzero", reporting a **false RESOLVED**.
5. The fix: **`band = max(floor, twin)`**, where `floor` is the propagated standard error of every median a reading is built from, sub-floored at the **measured** lattice quantum (`crates/boyko_app/tests/vg_occ_split_timing.rs:834` for the twin term, `:867` for the floor, `:871-892` for the quantum).
6. **The lattice is measured per sitting, never written down.** `timestampPeriod` is **1.0 ns** on this vendor and is the tick→ns **SCALE, not the counter increment** (`vg_occ_split_timing.rs:879-881`); flooring a band at it silences the alarm while leaving the false win. The odd-budget sitting measured **32 ns**; two prose sites in that same file still say 16 ns (`:138`, `:881`) — the 16 came from an *even* frame budget, where every median is the mean of two middle samples and can land on a half-tick. **The discrepancy is exactly why the number is computed by `measured_quantum_ns` at run time and never hard-coded**, and it is logged here as known in-tree doc-rot rather than silently resolved in one direction.
7. **An even sample budget puts medians off the lattice.** `DEFAULT_BENCH_FRAMES = 221` is odd *deliberately* (`vg_occ_split_timing.rs:301-306`), and removing the bias also removed — unplanned — the twin's degeneracy. **Consequence for this plan: `WINDOW` becomes 121, not 120.**

---

## Key decisions

### D1 — Emission: two gates, no allocation, no lock, one 16 B store

`zone!(HANDLE)` expands (feature on, site tier ≤ `GLOBAL_TIER`) to `let _z = ZoneGuard::open(&HANDLE);` — one `Acquire` load of a `CachePadded` global `ARM_MASK`, one statically-predicted-not-taken `bt`, one `rdtsc`; on `Drop`, one branch, one `rdtsc`, one 16 B store, one `Release` cursor store. Above the tier ceiling, or with the feature off, **the macro expands to nothing at all** — not to a use of its argument.

**Why the mask load is `Acquire`, not `Relaxed` (F11).** `ARM_MASK` gates the lane `buf` pointer, which is published `Release` at first arm. Rev 2 loaded the mask `Relaxed` and then loaded `buf` `Acquire`; the abstract machine then permits observing a set mask together with a stale null `buf`, and the hot path stores 16 B through it with no null test. **On x86-64 an `Acquire` load of an aligned word is the same single `mov` as a `Relaxed` one** — there is no fence, so the correction costs zero instructions. Rev 2's stated reason for `Relaxed` ("`Acquire` would forbid nothing while costing a fence off x86") had the cost backwards and the ordering obligation missed. The publication order is pinned in `arm()`: **slab → every `buf` (`Release`) → `ARM_MASK` (`Release`), in that order, always.**

**Why this shape.** NanoLog/Quill measure 7-9 ns with exactly it; spdlog measures 242 ns with the same asynchrony but caller-side formatting — the delta is entirely "do no work at the call site". At 400 zones × 60 Hz, 12 ns costs 0.03 % of a frame; 250 ns would cost 6 ms/s. The gate order (`const` tier ceiling `&&` runtime mask) is `log!`'s verified expansion: short-circuit `&&` over a `const` guarantees the arm and its operands vanish.

**Rejected.** `tracing` / `log` (third-party; `tracing`'s disabled check is a static callsite + two atomic loads, and its layers are `Box<dyn Layer>`). `Instant::now()` — **20-30 ns per pair** (`profile_spawn.rs:229-230`), which alone is 2× the whole open+close budget. `thread_local!` rings (TLS destructors at thread exit — the canonical lock-free-logger bug; the engine already has a pool-owned lane index). An `AtomicBool::swap` once-latch in a reader (`crates/boyko_render/src/render_path_config.rs:311-313` executes an RMW on a shared line every frame forever once its condition holds).

**Trade-off.** A `mem::forget`ed guard loses its sample silently in release; `ZoneGuard` is `#[must_use]` and a debug-only TLS depth counter (D3a) catches it in debug.

**Second trade-off, from the zero-instruction fix (F8).** Because the folded expansion names nothing, a typo'd zone identifier at a `Deep` site is invisible in a retail build. CI builds **both** tiers (rung 14), so the `Deep` leg is the one that catches it; the retail leg cannot and does not claim to.

### D2 — Lane taxonomy matches the ACTUAL thread topology; the lane is resolved ONCE per thread, not per emission

The engine has no present thread and no asset thread. The real hazard is that **the host thread is `UNATTACHED` outside `install`** (`tls.rs:29`) and therefore collapses onto lane 0 — worker 0's lane — precisely while it drives the post-present GPU readback.

```
lane 0..63   workers            (id from CURRENT_WORKER_ID)
lane 64      dispatcher         (WORKER_ID_DISPATCHER, i.e. host thread INSIDE install)
lane 65      host               (claimed by the runner at boot; host thread OUTSIDE install)
lane 66..67  spare claimable
lane 0xFFFF  LANE_UNCLAIMED     (emission is refused and counted)
LANE_COUNT = 68
```

**Rev 2 specified lane resolution twice, incompatibly** (F12): A1 step 4 said "lane from a TLS `Cell<u16>`, one load", D2 said "worker id `< 64` → that lane; `WORKER_ID_DISPATCHER` → 64; else the TLS-claimed lane; else drop" — different mechanisms with different costs — and nothing in the integration table set the TLS for workers, so *every worker would have resolved "unclaimed" and dropped*.

**Single specification.** `boyko_utils::profiling_abi::PROFILER_LANE: Cell<u16>` defaults to `LANE_UNCLAIMED`. It is **written once per thread**, at the three sites that know:

| Site | Value | File |
|---|---|---|
| `worker_main` entry, beside `set_current_worker_id` | the worker id | `crates/boyko_threadpool/src/tls.rs` + `worker.rs` |
| `ThreadPool::install` entry (and restore on exit) | `LANE_DISPATCHER` (64) | `crates/boyko_threadpool/src/thread_pool.rs` |
| `claim_profiler_lane()` from the runner at boot | `LANE_HOST` (65), else a spare, else `None` | `boyko_app::runner` |

Emission is then **one TLS load + one compare against `LANE_UNCLAIMED`**. The three-branch worker-id derivation is *initialisation*, performed once, not per sample. One OS thread may hold two lane identities over its life (dispatcher inside `install`, host outside); this is sound because the thread is serial, so each lane still has exactly one writer, and samples carry absolute TSC so the timeline joins without an epoch.

**Rejected.** An MPSC fallback lane (a second ring type, a second drain path, for zero threads). Widening `current_worker_id_or_dispatcher_lane` (it is the event system's contract; changing its `UNATTACHED → 0` sentinel would move event traffic).

### D3 — CPU clock: `rdtsc`, with a CV-reported calibration and an epoch-break detector

`clock::now()` → `_rdtsc()` when `CPUID.80000007H:EDX[8]` (invariant TSC) is set; a QPC-derived tick otherwise, with `boyko-W9207` and a raised quantum. At **arm** (a setup call, `debug_assert!(!is_in_system_run())`), `calibrate()` runs 16 probe pairs over a bounded `CALIB_WINDOW_MS = 20` window, discards probes whose `(rdtsc, Instant)` disagreement exceeds `1.5 × min_disagreement` (Tracy's rejection sampler), and publishes `ticks_per_ns` with **`calib_cv`** and `calib_rejected`.

**Why CV and not the worst probe:** peak-to-peak grows with `n` and cannot reproduce itself; attaching the worst-of-N probe to every printed nanosecond would be the same defect with the opposite verdict.

**Epoch breaks (X22, session-scale).** A game session is hours; suspend/resume and some power transitions move the TSC. The drain compares the frame's elapsed ticks against `MAX_PLAUSIBLE_FRAME_TICKS`; on violation it discards the in-flight window, bumps `epoch`, counts `epoch_breaks`, emits `W9216` once per break and re-runs `#[cold] calibrate()`. **No sample crosses a break, and `resolve` refuses any leg whose `epoch` differs from its partner's** (C4).

**Trade-off.** `rdtsc` is not serializing; the OoO engine may move instructions across a bracket. Consequence, printed as a field: the CPU channel's **quantum** is the measured `__cpu_null` median, and no span shorter than it is reported as a number. (Unlike its GPU sibling, `__cpu_null` is *not* measured to be zero — see D11a.)

#### D3a — `depth` is debug-only and lives in TLS, not in the lane

Rev 1 had `depth: u16` as a plain field inside a `static` — mutating it through `&'static` without `UnsafeCell` is UB, and no hot-path step incremented it. Removed. Nesting is reconstructed at drain (a region is single-writer, so its samples form an exact stack). Forgotten-guard detection is `#[cfg(debug_assertions)] OPEN_DEPTH: Cell<u16>` in TLS — zero release cost, no UB — and the drain's `debug_assert!(OPEN_DEPTH == 0)` becomes meaningful. `capacity` is a `const`, not a field.

### D4 — GPU readback is availability-polled and N-frames deferred; `WAIT_BIT` is made UNREPRESENTABLE

```rust
/// The verb takes NO flags parameter. The Vulkan implementation's flag word is a
/// private `const` const-asserted to exclude WAIT_BIT, so a blocking read is a
/// COMPILE error, not a grep result (G2a).
fn read_query_pool_pairs_available(
    &self, pool: &A::QueryPool, pair_count: u32,
    scratch: &mut [u64],                 // len >= 4 * pair_count (value + availability per query)
    out_begin_ticks: &mut [u64], out_dur_ticks: &mut [u64],
    out_available: &mut [u8],            // one byte per pair: 1 iff BOTH queries available
) -> Result<(), Self::Error>;
```

```rust
const GPU_ZONE_QUERY_FLAGS: u32 =
    VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WITH_AVAILABILITY_BIT;   // 0x1 | 0x4
const _: () = assert!(GPU_ZONE_QUERY_FLAGS & VK_QUERY_RESULT_WAIT_BIT == 0);
```

`VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x0000_0004` (rev 1 said `0x20`, which is undefined; `0x10` is `WITH_STATUS_BIT_KHR`). `VK_NOT_READY` maps to `Ok(())` with the corresponding availability bits **clear** — a normal outcome, not an error. The availability output is a **byte slice, not a `u128`** — no fixed-width wall.

**Why the const-assert and not a source gate (F3).** Rev 2's G2a grepped `gpu_zone.rs` and `profiling/**` for `WAIT_BIT`. The verb's *body* must live in `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs`, beside its three siblings `fetch_query_pair_ticks` / `fetch_query_pair_stamps` (`:1249`) — **a file the gate's scope structurally excludes**. That is the `-ValidationOn` failure shape the plan itself cites: a mechanical check whose scope excludes the defect. And the behavioural red is unavailable, because a blocking read **hangs**, and a hang is not a showable red in this repo (`vb_bench_totality_gate.rs:44-53`). Making the flag word a checked `const` converts the red into a build failure. The source gate is *kept as well*, but re-scoped: it asserts that **the set of files naming `vkGetQueryPoolResults` equals a pinned list**, so a new blocking reader in a new file fails the gate by existing.

`GPU_RING_DEPTH = 4 > FRAMES_IN_FLIGHT = 2`. A frame slot retires when every bracketed pair is available, or on the deadline in D4a.

**Why.** Tracy polls with the availability bit and breaks at the first unavailable query; Bevy resolves into a readback buffer and picks it up via `map_async` + `AtomicBool(Release)`. Neither blocks. The two hang classes documented at `gpu_timing.rs:186-203` and `:575-584` exist *only* because the reader blocks — removing the block closes both structurally, and with them the reason the three collectors are separate at all.

**Rejected.** Keeping `WAIT_BIT` + widening the totality epilogue (a device-side patch for a host-side mistake: it records two extra timestamps per unbracketed pass into the stream being measured, and makes termination depend on recorder discipline forever). `VK_QUERY_RESULT_PARTIAL_BIT` (the spec makes an unavailable result *undefined*, not zero).

**Trade-off.** Results arrive 2-4 frames late; a frame is `Pending` until its slot retires. Live display of the current frame's GPU cost is impossible — this is the mechanical source of C-IV's latency table.

#### D4a — `fence_seen` is derived from `RenderEpoch`, and the deadline is epoch-based (F13, F28)

Rev 2's `FrameSlot.fence_seen: AtomicBool` had **no source**: it named `frame_driver.rs:265` as "the fence gate … which is signalled", but that is `submission_epoch()`, a *submit* counter, and the tree has no non-blocking fence-status verb at all.

The derivation the counter's own doc supplies (`frame_driver.rs:255-262`) is used instead:

```
slot.submit_epoch = RenderEpoch at record time
fence_seen(slot)  = RenderEpoch >= slot.submit_epoch + FRAMES_IN_FLIGHT
```

`RenderEpoch` is already an ECS `Resource` (`crates/boyko_render/src/asset_refcount.rs:55`) written by the host every frame at `crates/boyko_app/src/runner.rs:1319`, one line above the ECS frame call. The retire step reads it. **No new RHI verb, no fence poll, no block.** The retire seam is the same one the asset system already trusts for freeing GPU-referenced memory.

**Termination when frames STOP** (shutdown, minimise, device-lost — F28). A frame-counted grace cannot fire if no frames run. The deadline is therefore two-horned and the second horn is host-driven: `Profiler::flush_gpu(&mut world, &recorder)` is called on the runner's teardown path (`crates/boyko_app/src/runner.rs:261` already force-drains host-owned per-frame resources there) and force-retires every in-flight slot as `Partial`, labelling unavailable pairs `LOST` and counting `gpu_slots_abandoned`. The count is **release-live** and reported; rev 2 lost up to `GPU_RING_DEPTH` slots silently.

### D5 — The witness survives as a per-pair mark array with a single seal; `__gpu_null` is DELETED

`AtomicU128` does not exist (not stable, not nightly; zero occurrences in the tree), and a hand-rolled 128-bit atomic is `cmpxchg16b`, a full RMW — not the cheap `Release` store the ordering argument assumes. Representation instead:

```rust
struct FrameSlot {
    marks: UnsafeCell<[u8; MAX_GPU_PAIRS]>,  // bit0 = begun, bit1 = ended; single producer
    seal:  AtomicU32,                        // the ONE release edge: stores `frame` after marks
    ...
}
```

The recorder writes marks (plain stores; exactly one thread per slot), then `seal.store(frame, Release)`. Retire does `seal.load(Acquire)`; if it equals the expected frame, the marks are visible. This scales to any pair count with no bitmask width wall and costs a plain byte store instead of an atomic OR.

Label is the 2×2 over (witness, availability):

| begun | ended | available at deadline | label |
|---|---|---|---|
| 1 | 1 | yes | `MEASURED` |
| 0 | 0 | – | `NOT_BRACKETED` (this leg does not run that pass) |
| 1 | 0 | – | `TORN` (recorder bug) |
| 1 | 1 | no | `LOST` → **NOT RESOLVED, no number printed** |

**Why the witness is still needed:** availability answers "the GPU wrote this query", not "the recorder bracketed this pass". A pass that never ran and a pass whose queries were never reported are both `available == 0` and mean opposite things. `gpu_timing.rs:432-445`'s argument is unchanged: a duration cannot distinguish a free pass from a filled one, and a begin-offset rule is a heuristic under mixed TOP/BOTTOM stages.

**`__gpu_null` is deleted (F6).** Rev 2 promoted `write_zero_pair`'s mechanism — two back-to-back `BOTTOM` stamps — to "the quantum probe". **Measured on this box, two adjacent `BOTTOM_OF_PIPE` stamps with no command between them report the same value: the probe reads 0, every time.** It is the same defect VG R3 P4-6 found in its own first design ("a strict order where two adjacent BOTTOM stamps cannot provide one"). A probe that is measured-inert is not a probe; keeping it would make D11's `BELOW QUANTUM` guard protect nothing on the GPU channel and reduce `resolve`'s `max(floor, quantum)` to `floor`. The GPU quantum is obtained the way the tree obtains it — see D11a.

`LOST` remains a state the old design could not express — it hung instead. **`LOST` is counted at the site and reported once per window with its count; it does not print per pair** (F20: rev 2's `emit_diag(W9205) per LOST` put up to 128 `eprintln!`s, each locking stderr and formatting, inside a per-frame path — the exact rule the plan applies to lane overflow and then abandoned here).

### D6 — Zone identity: a dense `u16` minted once, one registry, no strings on the emission path, exhaustion NON-terminal

```rust
declare_zone!(VB_EARLY_RASTER,
    name = "vb_early_raster", channel = Channel::GpuPass, kind = ZoneKind::Span,
    stage = GpuStage::BottomOfPipe, group = PartitionGroup::VbRun,
    scope = Scope::Render, tier = ZoneTier::Dev);
```

expands to `pub static VB_EARLY_RASTER: ZoneHandle { desc: &'static ZoneDesc, id: AtomicU16 }`.

**Minting — a total order over real values (F9).** Rev 2 wrote *"the winner stores the desc pointer into `REGISTRY[n]`, **then** `fetch_add`s the global counter"*, in which `n` does not yet exist. Executable sequence:

```
1. CAS handle.id: UNASSIGNED -> RESERVED        (Acquire on success, Relaxed on failure)
      loser -> #[cold] spin until id != RESERVED; return it
2. n = ENGINE_ID_NEXT.fetch_add(1, Relaxed)     // n now exists
3. if n >= ENGINE_ZONE_SLOTS {
       ENGINE_ID_NEXT.fetch_sub(1, Relaxed);    // monotone reservation restored, no id leaked
       handle.id.store(DISABLED, Release);
       drops.zones_refused += 1; W9201 once;  return DISABLED
   }
4. REGISTRY[n].store(desc_ptr, Release)         // the desc is published FIRST
5. handle.id.store(n, Release)                  // the id is published LAST
```

Rev 1's reserve-then-CAS leaked a counter value per lost race, making the id space sparse and firing exhaustion early; that fix is retained, now in an executable order.

**Ordering, specified ONCE (F10).** Rev 2 gave `AcqRel`/`Acquire` in the multithreading table and `Relaxed` in D6/A1. The single truth:

- `handle.id` — store `Release` (step 5), **load `Relaxed` on the emission path**. Sound because *the emitter never dereferences a desc*: it stores a bare `u16` into the sample.
- `REGISTRY[n]` — store `Release` (step 4), load `Acquire` at drain / report. This is the **only** desc edge. A drain that reads `REGISTRY[n]`'s stored value with `Acquire` synchronises-with the registrant's `Release`, and every byte of the desc was written before it. That holds whether or not the emitter is the registrant, which is what makes the emitter's `Relaxed` id load safe.

**One registry, one truth.** `static REGISTRY: [AtomicPtr<ZoneDesc>; ZONE_ID_SPACE]`. The `Profiler` Resource holds **no desc mirror** (rev 1 had two); the reporter reads `REGISTRY`.

System zones are pre-registered at `ScheduleBuilder::try_build` **when `GLOBAL_TIER >= Dev`**, so their emission path never takes the registration branch (the branch is still emitted; it is statically predicted not-taken and never taken).

**Exhaustion is NON-terminal (F5, C-III).** Rev 2 mirrored `query_type_registry.rs:124-144`'s terminal `E9201`. That precedent does not transfer: a query-shape registry is answering a *semantic* question, where a missing entry is a wrong answer; a zone registry is answering a *measurement* question, where a missing entry is a missing measurement. And the arithmetic makes the terminal form dangerous: `MAX_SYSTEMS_PER_SCHEDULE = 1024` (`schedule_builder.rs:70`), an `App` runs at least `Startup` + `Fixed` + `Main`, per-system minting was unconditional, and the feature is default-on — **so a legal app that never asked to profile could panic at build time.** Exhaustion now yields `ZoneId::DISABLED`, increments `zones_refused`, and emits `W9201` once. `W9208` still fires once at 90 % occupancy. G11 is the gate that makes the counter non-vacuous.

**Id space, sized against the kernel's own cap:**

| Range | Dev tier | Retail (`Always`) | Contents |
|---|---|---|---|
| `0 .. ENGINE_ZONE_SLOTS` | 4096 | 256 | `declare_zone!` sites, including ≤ 1024 systems × up to 3 schedules |
| `ENGINE_ZONE_SLOTS .. ENGINE_ZONE_SLOTS + dyn_budget` | `dyn_budget` ≤ 3072, default 256 | default 0 | `register_zone` (D19) |

`ZONE_ID_SPACE = ENGINE_ZONE_SLOTS + MAX_DYN_BUDGET`. The two ranges have **separate counters**, so a game exhausting its budget cannot consume an engine id (G11).

**Rejected.** defmt-style linker-section interning (the consecutive-address property is an ELF linker-script artifact; this box is windows-gnu/PE-COFF). A fixed `#[repr(u32)]` enum per subsystem (that *is* `VbTimedPass`, whose widening hazard we are removing).

### D7 — The stage table becomes a per-zone declaration, and partition sums are CHECKED — per frame, never over medians

`ZoneDesc.stage: GpuStage` and `ZoneDesc.group: PartitionGroup`. The reporter refuses to sum a group unless **every** member declares `BottomOfPipe` and their intervals are non-overlapping and contained in the group's run bracket; otherwise it prints members individually with `sum = NOT_VALID (mixed stage)`.

**Why.** `begin_stage`'s argument (`gpu_timing.rs:333-365`) is correct and currently enforced by nobody: consecutive `BOTTOM` stamps are prefix-completion times, prefixes nest, so their intervals exactly partition the span; a `TOP` stamp recorded *after* a `BOTTOM` stamp may legally report an earlier time. Today `froxel_total_ns` sums three independent brackets and discloses it only in a prose `NOTE:`.

**New in rev 3, from P4-6 fact 3:** the sum is formed **per frame, then reduced** — `median_f(Σ_members)`, never `Σ_members(median_f)`. `median(a) + median(b) ≠ median(a + b)`, and in P4-6 that inequality was crossed by 144-240 ns on a real reading. The reporter has **no** API that adds two reduced statistics; the addition happens in the frame-major row, which is the layout that makes it a single sequential pass (D8).

**Trade-off.** VB-P1d slots 0/1/2 stay `TopOfPipe` and can therefore never join a partition group. Correct — they never could.

### D8 — Storage: a `Resource`-owned FRAME-MAJOR SoA store on `VmReservation`; the stride is fixed at arm

**Layout fork, decided with numbers — recomputed (F15).** Rev 2's table said "≤ 256 lines ≈ 16 KiB" and omitted the `label` column entirely. Correct arithmetic, per frame row, at stride `Z`:

| Column | Width | Bytes at `Z = 1024` | Lines |
|---|---|---|---|
| `total` | `u64` | 8192 | 128 |
| `count` | `u16` | 2048 | 32 |
| `min` | `u32` | 4096 | 64 |
| `max` | `u32` | 4096 | 64 |
| `label` | `u8` | 1024 | 16 |
| **row total** | **19 B/zone** | **19 456 B ≈ 19 KiB** | **304** |

| Layout | Drain (per frame, hot) | Reporter reduction (cold, once) |
|---|---|---|
| zone-major `[zone*W + f]` | ~400 live zones × 5 columns = **2000 distinct lines ≈ 125 KiB**, far over L1d | sequential per zone |
| **frame-major `[f*Z + zone]`** | **304 lines ≈ 19 KiB** at `Z = 1024` | constant-stride gather (stride `19·Z` split per column), `WINDOW` reads per zone per column, hardware stride prefetcher applies |

**Frame-major wins by ~6.6× on the frequent side**, and the strided side runs `#[cold]` once per report. **Decided — but the L1d claim is now qualified honestly:** 19 KiB of columns plus the drain's ~6.4 KiB of sequential lane reads is 25.4 KiB against a 32 KiB L1d. It fits; it is not comfortable. At `Z = 2048` it does not fit, which is exactly what `W9211` reports.

**Arm-time `zone_stride` (X5).** `Z = ENGINE_ZONE_SLOTS + armed_dyn_budget`, fixed at `arm` and const for the session. `arm` twice with a different geometry ⇒ `E9213`. Above the L1d threshold `arm` still succeeds and emits `W9211` naming the measured working set — a game may legitimately want 2000 zones and pay for them, but it will be told.

**`WINDOW = 121`, not 120** (P4-6 fact 7). An even window makes every median the mean of the two middle samples — a value no frame produced, sitting half a lattice tick off. That is precisely how the 16 ns lattice was first mis-derived. 121 frames ≈ 2.02 s at 60 Hz. Column bytes: `19 × 1024 × 121 = 2.35 MiB` at the dev default.

**Backing store.** One `VmReservation` (`crates/boyko_ecs/src/ecs/memory/vm.rs:99`) reserved and committed at first arm, owned by the `Profiler` `Resource`. Every column and the lane sample slab are sub-slices of it. **No `Box<[T]>`, no `Vec`** (F7).

**Transport control blocks.** `static LANES: [ZoneLane; 68]` in `.bss` (17 KiB with the two-region split, D19). Each region's `buf: AtomicPtr<Sample>` is published `Release` once at first arm and never nulled.

**Multi-world.** The rings are process-global; worlds are not. **v1 binds the profiler to exactly one world**: `ProfilerPlugin::build` records the `WorldId` in a global; a second registration is `boyko-E9204`. Enforced at bind time, not assumed.

### D9 — Concurrency = STATIC compatibility vs OBSERVED interval overlap, at the kernel's own system bound

Rev 1 could not compute its own headline. Rev 2 could, but at `MAX_SYSTEMS = 512` against a kernel cap of **1024** (`schedule_builder.rs:70`), with no counter for what it truncated — M8's exact defect at 2× the bound (F5). And its `intervals` write was an **assignment** to one slot per `(frame, system)`, while the host frame is *"Time → events → Fixed×N → Main"* (`crates/boyko_app/src/runner.rs:943`), so a system in `Fixed` overwrote itself N−1 times per frame (F19b). And `sys` was not derivable — `Sample` carries `zone: u16` only (F19c).

All three fixed:

- **Declared** = the static compatibility matrix, snapshotted from `ConflictGraph` at arm, at `MAX_SYSTEMS = MAX_SYSTEMS_PER_SCHEDULE = 1024`: `compat`, 1024×1024 bits = **128 KiB**. Pair `(i,j)` is compatible iff no access conflict and no ordering edge in either direction. The snapshot covers the **one schedule named in `ProfilerConfig::analysed_schedule`**; systems in other schedules are counted in `systems_unanalysed`, never silently dropped.
- **Observed** = an **append ring**, not an assignment: `intervals: [Interval; OVERLAP_FRAMES × INTERVALS_PER_FRAME]` with `OVERLAP_FRAMES = 8`, `INTERVALS_PER_FRAME = 2048`, `Interval { begin: u64, dur: u32, sys: u16, occ: u16 }` = 16 B → **256 KiB**. A system running N times per frame appends N intervals; overflow increments `intervals_dropped`.
- **`sys` resolution**: `ZoneDesc.system_index: u16` is set at mint time in `try_build` (the builder knows the index), and `arm` builds a `sys_of: [u16; zone_stride]` side table — 2 KiB, L1-resident, one indexed load per system-tagged sample at drain. The same shape as `hist_of` (D22).
- `RoundRecord { frame: u32, round: u16, dispatched: u16, begin: u64, end: u64 }` = 24 B keeps dispatch *shape* only (rounds per frame, wave width, round span). No membership mask, hence **no truncation and no silent wrong answer**. `MAX_ROUNDS_PER_FRAME = 32`; overflow is counted and reported.

`ConcurrencyReport` prints, per compatible pair that both ran: `declared=1 observed_frac=x.xx`, plus the aggregate **serialisation index** = 1 − (Σ observed overlap / Σ declared-compatible-and-both-ran).

**All of D9 is behind `feature = "profiling-analysis"` (default ON in dev, OFF at retail).** `compat` + `intervals` = 384 KiB, which a shipping title has no use for.

### D10 — Fully in-house. No Tracy stream, no Tracy protocol, v1 or v2

1. `tracy-client` is a C++ client, a build script and a TCP server process — the largest possible dependency against a standing zero-third-party stance.
2. **Tracy's wire format cannot represent the one property this system exists for.** `NOT RESOLVED`, `LOST`, a band, a measured quantum — none is expressible as a Tracy zone. Exporting would render them as durations, i.e. launder unresolvable deltas back into numbers.
3. Tracy's genuine inventions — availability-polled collection, a rejection-sampled calibration — are techniques, and we take them (D4, D3). The protocol is not the technique.

**Concession.** No free viewer. The dev artifact is flat TOML with `schema_version`; the session artifact is the binary stream (D23) plus its in-tree decoder. A v1.2 optional exporter may emit Chrome-trace JSON containing **only `MEASURED` rows** — the dropping is the exporter's purpose, not its limitation.

### D11 — The band is `max(floor, twin)`. A `Floor` is cross-process. A `Twin` is in-sitting. Neither is a quantum

Rev 1 substituted an empty bracket, measured within one session, at 1σ. Rev 2 removed the first substitution and **kept the other two** in `Floor::from_aa_control(control: &LegSummary, sigma: f64)` — a single in-sitting control with a caller-supplied sigma (F4), while asserting *"`Floor` is a type with no cheap constructor"*. And `resolve` accepted **any** `Floor` for **any** pair of legs, so a floor measured on the VB cull class could license a delta on the SV0 class — verbatim what `vg_decidability_floor.rs:28-30` forbids.

**Three distinct quantities, never conflated:**

| Quantity | What it is | How measured | Where it appears |
|---|---|---|---|
| **Quantum** | the instrument's own resolution | CPU: `__cpu_null` median. GPU: `measured_quantum_ns` — the GCD of every timestamp-derived value published **this sitting** (D11a) | printed beside every number; a span below its channel's quantum prints `BELOW QUANTUM`, never a value |
| **Floor** | the smallest defensible *relative* delta for **this workload, this box, this protocol** | `FLOOR_SIGMA = 3.0 × CV` of the **workload under test**, across `SESSIONS = 7` separate processes, `REPEATS = 3`, all three repetition floors printed and never averaged — the `vg_decidability_floor.rs:27-73` protocol verbatim | one term of the band |
| **Twin** | ongoing DRIFT during the sitting | the interleaved zero control, reduced by `max(\|median\|, p90\|·\|)` — `vg_occ_split_timing.rs:834` | the other term of the band |

```rust
pub struct WorkloadTag(u64);   // hash of the subscribed zone-id set + the config identity

pub struct Floor { rel: f64, workload: WorkloadTag, sessions: u32, repeats: u32, path: PathBuf }
impl Floor {
    pub fn from_session_file(path: &Path) -> io::Result<Floor>;   // THE ONLY constructor
}
// deleted in rev 3: Floor::from_aa_control(control, sigma)  -- one sitting, caller-chosen sigma
// never existed:    Floor::from_quantum

pub struct Twin { ticks: u64, rounds: u32, workload: WorkloadTag }
impl Twin { pub fn from_zero_control(control: &LegSummary) -> Twin; }   // no sigma parameter

pub fn resolve(a: &LegSummary, b: &LegSummary, floor: &Floor, twin: &Twin) -> Contrast;
```

`resolve` computes

```
band = max( floor.rel * |median_a| ,           // cross-process 3σ CV, this workload
            twin.ticks ,                       // in-sitting drift
            se_floor(a, b) ,                   // propagated SE of every median a reading is built from
            quantum_of_channel )               // sub-floor; never the whole band
```

and returns `NotResolved` — **with the delta fields still populated** — on any of:

| `NotResolvedReason` | Trigger |
|---|---|
| `BelowBand` | `\|median_delta\| <= band` |
| `FloorWorkloadMismatch` | `floor.workload != a.workload` — the check rev 2 carried the fields for and never made (F4) |
| `TwinWorkloadMismatch` | `twin.workload != a.workload` |
| `WindowIncomplete` | either leg's window carried a drop of any class (C4/X8) |
| `EpochBreak` | the legs' `epoch` values differ (D3) |
| `LabelNotMeasured` | any subscribed GPU zone in either leg is `LOST` / `TORN` / `NOT_BRACKETED` |

**`FLOOR_SIGMA = 3.0` is a `const`. There is no caller-supplied sigma anywhere in the API.**

**Contrast protocol: ABBA, never ABAB.** With `FRAMES_IN_FLIGHT == 2`, strict alternation aliases the A/B phase perfectly with the frame-in-flight slot — different pool, different UBO ring slot, different staging, forever. ABBA breaks the alias; the cancelled order bias is **reported** (`order_bias_ticks`), not hidden (`crates/boyko_app/tests/sv0_deferred_term_bench.rs:20-72`, generalised).

**No warm-up doctrine.** Warm-up 20 → 100 was tried and reverted as a measured negative (`crates/boyko_app/src/runner.rs:158-172`): the ramp is ongoing drift, not a settling transient. Instead every window prints `median_first_half` / `median_second_half`, so drift is visible rather than assumed away.

#### D11a — The GPU quantum is measured per sitting and never written down

With `__gpu_null` deleted (D5/F6), the GPU quantum comes from the tree's own estimator, generalised into the reporter: **the GCD of every timestamp-derived value the sitting published** (`vg_occ_split_timing.rs:871-892`). Three properties are carried verbatim because each was earned:

1. **`VkPhysicalDeviceLimits::timestampPeriod` is NOT this number.** It is the tick→ns *scale* (1.0 ns on this vendor), not the counter *increment*. `vg_occ_split_timing.rs:879-881`: flooring a band at `period × 1 tick` *"would satisfy every arithmetic check while silencing the alarm"*.
2. **Means are excluded from the GCD.** An arithmetic mean of `n` lattice values is not itself on the lattice (`:885-886`). Only odd-`n` medians are.
3. **The number is computed, not hard-coded.** The same file's prose says 16 ns at `:138` and `:881`; the odd-budget sitting measured **32 ns**. Whichever is right today, a constant in this plan would be wrong tomorrow. The reporter prints the quantum it measured and the count of values it was derived from; if the sitting published no nonzero value, the quantum is `UNKNOWN` and every GPU number in that report is `NOT RESOLVED`.

### D12 — Present mode becomes configurable; wall clock is demoted to a labelled, probed observation

`PresentModeConfig { Fifo, Immediate }` (`Mailbox` is declared and returns `Unsupported` until a harness needs it — one code path, not three). Default `Fifo`, so no golden pin moves. Support is **probed** with the existing `present_mode_supported` (`present/surface.rs:218`, used at `present/swapchain.rs:164`) and the *resolved* mode is recorded in the artifact; an unsupported request falls back to `Fifo` with a loud notice (the `BootError::ValidationUnavailable` precedent: refuse or announce, never silently degrade).

The `Frame` channel's wall clock always carries its bound: `frame_wall_ns=… bound=FIFO(refresh≈16.67ms)` or `bound=none`. **Even under `Immediate`, wall clock stays secondary**: the primary CPU number is the `__frame` span (D16), and the primary GPU number is the device-tick delta.

**Why at all.** While FIFO is unconditional no wall-clock gate can fail for GPU-side work, and this project treats a gate that cannot fail as a defect — the measured precedent being `-ValidationOn` reporting "clean, 0 messages" for all 22 pins while an illegal `mip_levels: 12` drew zero.

**Honest scope note, promoted out of the open questions (F-rung-8).** `Immediate` support on this box is **unproven**. If it is unsupported, rung 8's present-mode work reduces to *labelling* — the frame channel stays FIFO-bounded and non-decidable, and no new wall-clock gate becomes showable. The rung table says so, not only an open question.

### D13 — Counters and gauges are typed at the WINDOW level, so the wrong statistic is unrepresentable

`ZoneKind ∈ { Span, Counter, Gauge }`, and the accessors are kind-specific:

```rust
fn span(&self, id: ZoneId)    -> Option<SpanWindow<'_>>;
fn counter(&self, id: ZoneId) -> Option<CounterWindow<'_>>;   // None on wrong kind
fn gauge(&self, id: ZoneId)   -> Option<GaugeWindow<'_>>;
```

`rate_per_frame` exists only on `CounterWindow`; `median_frame_ticks` only on `SpanWindow`. Rev 1 put all three on one `ZoneWindow` and **panicked** on the wrong kind — a runtime panic in a library API against the repo's `Option` / `expect("invariant: ...")` convention.

flecs types this (`ecs_gauge_t` vs `ecs_counter_t`); Unreal types it; Bevy does not, which is exactly why "an average frame count would be nonsensical" is special-cased in a plugin.

**Counter authoring rules — `VbRecordProbe`'s three, promoted to contract** (`crates/boyko_rhi_vulkan/src/present/passes/vb.rs:86-100`, increment sites at `:107-156`):
1. **Counts originate AT the operation they count** — at the `vkCmd*` call, inside the cull loop — never re-derived on the host. *"A host that re-derives `scopes` from `GBufferScene::vb_occlusion_instances` agrees with itself no matter what this function did — the tautology this campaign has shipped as a gate five times."*
2. **Host memory, not a device buffer.** A device counter adds an allocation, a declared pass, a barrier, a fence wait and a decode to move a number already in a register — and changes the recorded command stream.
3. **What a counter cannot claim is a field of the artifact**, not a prose paragraph.

**Allocation counting.** The 19 zero-alloc gates each install a process-global allocator, which is why they can only be test binaries. **The profiler installs no global allocator.** An opt-in `profiling-alloc` feature in `boyko_app` installs a counting shim feeding the `Counter` channel; off by default, `#[cfg]`-excluded at retail tier, and its perturbation is stated in the artifact when on.

### D14 — Clock correlation is two-tier; tier 1 says `UNCORRELATED` rather than guessing

**Tier 1 (v1, mandatory, zero cost).** GPU spans on a device-tick axis anchored at the frame's first `BOTTOM` stamp; CPU spans on a TSC axis anchored at `update_with_delta` entry. Two lanes, declared unmeasured offset, artifact field `cpu_gpu_offset = UNCORRELATED`.

**Tier 2 (v1.1, gated on `VK_EXT_calibrated_timestamps`).** 32 probes at arm, acceptance threshold `min_deviation × 3/2`, recalibration each drain, `max_deviation_ns` published with every correlated number.

**Why defer.** Every question the audit found being asked is within-domain. The Khronos problem statement is why it cannot be faked: core Vulkan timestamps *"cannot be compared even across separate submits within the same run of an application, as power management events can reset the timer."* An uncalibrated cross-domain offset is not an approximation; it is a fabrication. **Rev 3 adds a second trigger to revisit:** an in-game overlay showing two axes (D25) will make users ask for one, and that request must be answered with v1.1 or with a refusal — never with an uncalibrated offset.

### D15 — Lane buffers are allocated once and NEVER freed; disarm is a mask store

Rev 1 freed the lane slab at disarm behind a quiescence argument that covered workers only. The stated guard was worse than absent: `is_in_system_run()` (`crates/boyko_threadpool/src/tls.rs:83`) reads **the calling thread's own TLS** and can never observe another thread. And the cited precedent does not transfer: `ThreadLaneWriter`'s `Sync` clause 2 (`events/event_buffer.rs:102-106`) rests on *"`update_events`, which takes `&mut EventDispatcher` — the `&mut` acts as the synchronisation point"*; a `static LANES` has no `&mut` to stand in for that clause.

**Decision: there is no free.** The `VmReservation` is created on the first `arm()` and lives for the process. `arm`/`disarm` only store `ARM_MASK` and reset cursors. `buf` is published once, `Release`, and never nulled. The use-after-free class is deleted by construction rather than argued away.

**Honest consequence:** "disarmed = a few KiB of BSS and nothing else" is true only *before the first arm*. After a first arm, disarmed resident cost is the full committed reservation. Stated in the artifact and in the target table.

### D16 — The instrument is outside its own primary number, and the number is defined across `Fixed×N` + `Main`

Rev 2 put the drain in `App::update` (`crates/boyko_ecs/src/ecs/core/app/app.rs:736`). **The windowed host does not call it**: `crates/boyko_app/src/runner.rs:1321` calls `app.update_with_delta(dt)` directly, and `App::update` (`:736-744`) merely computes a delta and forwards. In the only configuration that has a GPU channel, **the drain would never have run** — lanes fill, `overflow` climbs, no frame ever seals — and nothing in rev 2's gate list caught it, because the unit tests drive `App::update` (F2).

**The drain moves to the top of `App::update_with_delta` (`app.rs:655`), the single funnel both entry points share**, before step ① (`Time::advance_with`). `App::update` needs no change.

Second half of F2: rev 2's "primary CPU number" was *"the `Schedule::run` span"*, but `runner.rs:943` documents the frame as *"Time → events → Fixed×N → Main"* — **two schedules, and `Fixed` runs N times.** "The `Schedule::run` span" is not one interval, and "the drain is outside the primary number" was undefined across N+1 runs.

**Definition, stated once:**

| Zone | Bracket | Cardinality |
|---|---|---|
| `__frame` | `update_with_delta` entry (**after** the drain returns) → exit | 1 per frame — **this is the primary CPU number** |
| `__events` | step ③ `update_events` | 0 or 1 |
| `__fixed_step` | one `fixed.run(world)` inside step ④ | **N** per frame; `FrameRecord.fixed_steps` records N |
| `__main_run` | step ⑤ `schedule.run(world)` | 1 |
| `__drain`, `__report`, `__hist_fold`, `__telemetry_write` | the instrument's own work | outside `__frame` by construction |

`FrameRecord` carries `run_gross` (= `__frame`), `fixed_total` (Σ over the N substeps), `main_total`, `instrument_measured` and `instrument_estimated`. All are printed.

**`instrument` is split (F18).** Rev 2 defined `instrument = Σ __drain + __report + __cpu_null + zone_count × measured_zone_cost` and then printed `run_net = run_gross − instrument`. The last term is an **estimate from a different binary and profile**, injected into a per-frame number — in the document that refuses to print unresolvable deltas and that cites `median(off)+median(dur) ≠ median(off+dur)`. So:

- `instrument_measured` = Σ of the instrument's **own zones**, measured in-band this frame.
- `instrument_estimated` = `zone_count × zone_cost_ticks`, carrying `zone_cost_provenance` (bench id + `build_hash`).
- **`run_net = run_gross − instrument_measured_inside_frame`. The estimate is never subtracted from anything.** It is printed beside, labelled, with its provenance.

### D17 — Disarmed byte-identity is proved by a COMMAND CENSUS, not by image hashes, and the armed clause is an EQUALITY

`goldens/PINS.toml:3` pins SHA-256 of a dumped BMP. A `vkCmdResetQueryPool` plus two `vkCmdWriteTimestamp`s change zero pixels, so rev 1's G5 ("record one command on the disarmed path → pins move") was false as written.

**Mechanism:** `RecordCensus { profiling_cmds: u32, query_resets: u32, timestamps: u32, first_pair_of: [u16; MAX_GPU_PAIRS], recorded_pairs: u16 }`, a `&mut` host parameter threaded into the recorders and incremented **at the `vkCmd*` call sites** (D13 rule 1), exactly like `VbRecordProbe` (`present/passes/vb.rs:107-156`).

Two-sided gate:

- disarmed frame ⇒ `profiling_cmds == 0` (and every sub-counter 0);
- armed frame ⇒ **`timestamps == 2 × recorded_pairs` and `recorded_pairs == declared_bracket_count`** — an *equality against a host-side declared count*.

Rev 2's armed clause was `timestamps >= 2`, which **the instrument's own `__gpu_null` probe satisfied by itself** — a recorder that dropped every real bracket passed (F-gate-table). `__gpu_null` is now deleted and the clause is an equality, so the only way to pass is to record exactly the declared brackets.

**`first_pair_of` is the RECORD-ORDER WITNESS** (P4-6 fact 2). Timestamps cannot license a conclusion about record order — two stamps that resolve on the same tick say nothing about which `vkCmd*` came first. The census records, host-side at the call site, the order in which pairs were opened. Every claim in this system about *record order* reads `first_pair_of`; no claim about record order reads a timestamp.

Golden pins remain a *secondary* check on pixels, with the explicit note that they are structurally incapable of the command claim.

### D18 — `hostQueryReset` is an optimisation with a fully specified fallback

Enabling `VkPhysicalDeviceHostQueryResetFeatures` at device creation records **no commands** and changes no frame; it is a `pNext` bit, and the goldens are unaffected. It is enabled when the physical device advertises it.

Nothing establishes that this box's driver does (`ffi.rs:2716` shows the field exists and is never enabled). **The design does not depend on it.** Fallback, specified rather than named: a slot that retires without host reset sets `needs_cmd_reset`; slot recycling refuses that slot until an armed frame issues `vkCmdResetQueryPool` for it at the frame top — the exact site the current code already uses, outside any render scope, satisfying `VUID-vkCmdResetQueryPool-renderpass`. With `GPU_RING_DEPTH = 4` and `FRAMES_IN_FLIGHT = 2` there is always a clean slot, so the fallback never stalls. Host reset merely removes the one-frame recycle latency.

---

## Scope extension: the game-facing half

The owner's requirement is that games use this system, collecting as much data as possible, with maximum flexibility. What follows are the decisions that requirement forces, including the two places where **the requirement as literally stated is a bad idea for this engine and is refused with a reason**.

### D19 — Two authoring paths, ONE registry, ONE store — with a partitioned id space and a partitioned ring

| Path | Who | Cost | Tier-foldable |
|---|---|---|---|
| `declare_zone!` (static) | engine crates **and any Rust plugin crate downstream** | ≤ 12 ns | yes |
| `register_zone` → `DynZoneHandle` (dynamic) | zones defined by data / config / script / mods | ≤ 14 ns (≤ 18 ns across an FFI/script boundary) | **no** — a data zone has no compile-time tier |

`declare_zone!` is exported from `boyko_utils::profiling_abi` and re-exported through `boyko_ecs::prelude`, so **X1 needs no new mechanism at all**: a game plugin crate writes `declare_zone!` verbatim and pays the engine's price.

**The game path must not be able to degrade the engine path.** Two partitions, each with its own gate:

1. **Id space** (D6): `ENGINE_ID_NEXT` and `DYN_ID_NEXT` are separate counters over disjoint ranges. A game exhausting `dyn_budget` gets `W9210` and a refused registration; the engine's next `declare_zone!` still mints. **G11.**
2. **Ring capacity**: `ZoneLane` becomes **two SPSC regions** — `ENGINE` and `USER`. The region is a **compile-time constant of the emitting site** (the macro knows which it is), so there is no runtime branch. A runaway game scope fills the `USER` region and drops `USER` samples; `engine_overflow` stays 0. **G20 — the extension's headline gate.**

Cost, stated: lane control blocks 8.5 → 17 KiB `.bss`; per-region capacity 4096 → 2048 samples. 2048 samples at ~400 engine samples/frame is 5 frames of burst headroom, against a drain that runs every frame — argued sufficient, and `G4` makes any shortfall visible rather than silent.

### D20 — Runtime toggling: `ProfilingScope` is an ECS entity with an `IsEnabled` bit — projected by the DRAIN, because the kernel fires no observer

**`ARM_MASK: AtomicU64`** replaces rev 2's `CHANNEL_MASK: AtomicU32`. Identical instruction count on the hot path (a `bt` against a 64-bit word). Bit layout:

```
0..7    channels        SchedulerCpu, GpuPass, Counter, Frame, User0..3
8..31   engine scopes   Render, Physics, Input, Assets, UI, Audio, Net, …
32..63  game scopes     assigned by register_scope()
```

**The extension proposed an `IsEnabled` *observer* projecting into `ARM_MASK`. That mechanism does not exist and cannot be built without a kernel change.** `crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:77-88` documents the enable path as *"O(1) warm: no migration, no structural-generation bump, **no hook / observer fire**, no deferred drain"* — the absence of a fire is precisely what buys the O(1) toggle. Any design that "projects on the transition" is unimplementable here.

**Replacement — the projection is a step of the drain, not a system and not an observer.**

```
drain(world, ...):
    0. scope projection:  for b in registered_scopes:            // scope_count, typically < 16
           bit(b) = world.is_enabled::<ProfilingScope>(scope_entity[b])
       if bits != ARM_MASK { ARM_MASK.store(bits, Release) }      // one store only on change
    1. .. the sample drain ..
```

- **The ECS remains the single source of truth.** `ProfilingScope` is a component on an entity; the on/off state is the kernel's `IsEnabled` bit. That is the project's capability/state rule applied verbatim: capability = component presence, runtime on/off = the enable bit. There is no parallel mechanism, **no public mask setter**, no mirror and no dirty flag.
- **The cost is measured, not assumed.** `is_enabled::<T>` is documented at `enable_tag_api.rs:100-105` as O(1), *"≤ 5 ns"*. At 16 registered scopes that is ≤ 80 ns per frame; at the 64-scope maximum, ≤ 320 ns. It runs **inside `__drain`**, i.e. inside `instrument_measured` and outside `__frame` (D16), so it is disclosed rather than hidden. A `scope_scan` bench leg reports it.
- **No new system, no new `SystemSet` for control, no reconciliation system in the schedule.** Rev 3 rejects the extension's `ProfilerSet::Control` for the same reason it rejects `requires_dispatcher` on retire (F14): adding scheduled systems to the subsystem that measures scheduling perturbs its own product.
- **Latency is one frame and is documented.** A console command, a dev menu, a network handler and a save-file all reduce to `world.enable::<ProfilingScope>(e)` / `disable`; the effect appears at the next drain. **G12 asserts the *next* frame, two-sided.**

**Rejected: a scope hierarchy.** Parent/child scopes make the emission gate more than one `bt`. A game wanting hierarchy composes bits itself, in its own code, visibly.

### D21 — Compile-time tiers: what survives into a shipping game

```rust
#[repr(u8)] pub enum ZoneTier { Always = 0, Dev = 1, Deep = 2 }
pub const GLOBAL_TIER: ZoneTier = /* from option_env!("BOYKO_PROFILING_TIER"), default Deep */;
```

The macro's first gate is `const { site_tier as u8 <= GLOBAL_TIER as u8 } && ARM_MASK…`. A short-circuit `&&` over a `const false` deletes the arm **and its operands**, which is the `log!` property D1 relies on.

| Tier | Contents | Retail |
|---|---|---|
| `Always` | frame time, a small counter set, crash/telemetry-relevant gauges | **ships** |
| `Dev` | per-pass GPU zones, subsystem spans, histograms | folded out |
| `Deep` | per-system scheduler zones, round records, per-draw counters | folded out |

Orthogonally, `feature = "profiling-analysis"` gates the `compat` matrix, the `intervals` ring, `ConcurrencyReport`, the contrast machinery and the TOML writer — the parts a shipping title never runs. Retail = `Always` + analysis off ⇒ **≤ 1 MiB resident**.

Costs, stated: changing the tier rebuilds the workspace (it is an `option_env!` const, so `boyko_ecs` gains a `build.rs` emitting `cargo:rerun-if-env-changed`); ~12 KiB of dead `.bss` remains for folded `ZoneHandle` statics; and a typo in a `Deep` zone name is invisible in the retail leg (D1's second trade-off).

**G14 is two-sided**: an object-symbol census shows **no** reference to the recorder from a `Deep` site **and** an `Always` site in the same binary **does** reference it. A ceiling that folds everything passes the first clause and fails the second.

### D22 — Retention: three tiers, because a fixed ring cannot hold an hour (C-I)

| Tier | Structure | Horizon | Cost | Always on? |
|---|---|---|---|---|
| **A** | the frame-major ring | `WINDOW = 121` frames ≈ 2 s | 2.35 MiB at `Z = 1024` | yes when armed |
| **B** | lifetime accumulators — `{ total: u64, count: u64, max: u32, min: u32 }` per zone | the whole session | `24 × Z` = 24 KiB at `Z = 1024` | yes when armed |
| **C** | log-linear histograms — 3 mantissa bits, 192 buckets of `u16`, 400 B per slot | the whole session | `cfg.hist_slots × 400 B`; 25 KiB at 64 slots | opt-in |

Tier B is folded in **one sequential pass over the current frame's row**, which the drain just touched, so it is L1-warm. Tier C folds only for zones with a slot (`hist_of[z] != 0`, a `Z`-byte L1-resident map).

**No per-frame history beyond `WINDOW`, ever.** That is C-I's cost to the game side, and it is deliberate: per-frame retention over an hour is 216 000 rows × 19 B × `Z`, which is not a ring, it is a database.

**Bucket geometry is chosen against the measured floor band (4.7-14.3 %), not against a general requirement.** 3 mantissa bits ⇒ 6.25 % bucket width, the same order as the floor — which is exactly why **`resolve` does not consume histograms**. If a title needs tighter session quantiles the mantissa widens to 4 bits (384 buckets, 784 B/slot): a config, not a redesign.

Saturation is **counted** (`hist_saturations`), never silent: a `u16` bucket saturates at 65 535 samples, which for a per-frame zone is ~18 minutes in one bucket.

### D23 — Player telemetry: an append-only binary stream, window-granular, synchronous, no new thread

```
header (128 B, once per file): magic, schema_version, session_id, run_id, build_hash,
                               player_tag[16] (opaque; the engine never interprets it),
                               tier, zone_stride, window, ticks_per_ns, calib_cv
ZoneRow  (variable, once per zone per file): id, kind, unit, scope, name bytes
WindowRec(40 B, per subscribed zone per window): id, count, total, min, max,
                               median, p95, drops, epoch, fixed_elapsed_ns
```

One `write_all` per window (2 s), from a boot-allocated double buffer, on the dispatcher, `#[cold]`, **inside `instrument_measured`** (D16), so it is disclosed. Retail volume ≈ 2.9 MB/h. Rotation at `max_bytes`. A write error sets `telemetry = None`, counts `telemetry_write_errors` and emits `W9215` once — **never panics, never retries in-frame**.

**No second thread (X25), refused on the number.** 20-60 µs per 2 s is 0.36 % of one frame in 120. The engine's only threads stay the pool's. Named escalation trigger: `__telemetry_write` p95 > 200 µs on a real title, measured ⇒ hand the byte blob to `boyko_log`'s existing sink — **one thread for both subsystems, never two.**

**Loss bound: ≤ one window on a hard kill**, because there is no cross-window buffering. G15 proves that bound and states what it cannot cover.

**`fixed_elapsed_ns` = `FixedTime::elapsed()`** (`crates/boyko_ecs/src/ecs/core/time/fixed_time.rs:162`) is the kernel's own determinism witness, so a stream correlates with a replay at 8 B per record (X18).

### D24 — Drop accounting stays honest at session scale

- **(a) The clear is `fetch_sub(observed)`, not `store(0)`.** The drain loads `overflow`, accumulates it into a `u64`, then subtracts *exactly what it observed*. A producer increment between the load and the clear survives. `debug_assert!(observed <= region_capacity)`.
- **(b) Every drop class is attributed, and every class is attributed PER REGION**: `engine_overflow`, `user_overflow`, `unclaimed`, `late`, `rounds`, `intervals_dropped`, `gpu_budget`, `gpu_lost`, `gpu_slots_abandoned`, `zones_refused`, `dyn_registrations_refused`, `hist_saturations`, `telemetry_write_errors`, `epoch_breaks`, `systems_unanalysed`. All `u64`. All **release-live** — a reporting obligation that vanishes in release is the vacuous-gate pattern by another route.
- **(c) Non-wrap proof.** A `u32` region counter can gain at most one increment per refused sample per drain interval; one frame at 60 Hz cannot produce 2³² refusals from 68 lanes at 2048 slots each. Accumulated into `u64`, which at 10⁶ drops/s wraps in 585 000 years.
- **(d) `resolve` refuses a leg with any drop** (`WindowIncomplete`, D11). **This tightens the engine side:** a bench that drops now produces no number instead of a wrong one (X8).

### D25 — The game reads its own numbers from ECS systems — windowed, lagged, and NOT a message bus

`Res<Profiler>` is readable from any system. Two things make that safe and cheap:

- **`ProfiledZone(ZoneId)`** — a component resolving a name to an id **once at setup**, so a reader never calls the `#[cold]` `by_name`.
- **A printed latency table**, because the lag is structural, not incidental:

| Datum | Freshest available | Why |
|---|---|---|
| CPU spans, counters, gauges | frame **N−1** | the drain folds closed frames only (A2's live-frame cut) |
| GPU spans | frame **N−4 … N−2** | availability polling + `GPU_RING_DEPTH` + `RETIRE_GRACE_FRAMES` (D4) |
| lifetime / histogram | through N−1 | folded at the same drain |

**Ordering.** The retire step and the drain both run **outside the schedule** (D16, D4a), before any system executes, so every `Res<Profiler>` reader in the frame sees the same consistent snapshot with no intra-frame ordering edge and **no new `SystemSet`**. This is strictly better than the extension's `ProfilerSet::{Retire, Read}` pair, which would have needed a scheduling edge and a dispatcher-pinned system.

**Refused (X14, half):** *same-frame* counter readback as an inter-system message bus. It costs either a shared-line RMW on the emission path or a mid-frame drain, and the ECS already has events and resources for that. A game's own counters are ECS data the profiler **samples** (via `gauge!` once per frame); they are not data the profiler **stores** on the game's behalf.

**Supported (X14, half):** reading *windowed statistics* to drive LOD, dynamic resolution or quality scaling. That is a one-frame-stale median, which is what those controllers want anyway.

**Reference overlay** (`boyko_ui/src/profiling_overlay.rs`, rung 15) — allocation-free, gated by G19 with a positive control.

### D26 — Session identity in; cross-process aggregation and a live viewer out

**In:** `session_id`, `run_id`, `build_hash`, an opaque 16-byte `player_tag` the engine never interprets, and replay correlation via `FixedTime::elapsed()`. 44 B of header, 8 B per record.

**Out, argued:**
- *Cross-process / networked aggregation* — needs cross-machine clock correlation, which D14 refuses to fake on **one** machine. Re-entry condition: the merge is a tool over files that already share `session_id` + `fixed_elapsed_ns`; build it when two files exist that anyone wants merged.
- *Live network viewer / remote streaming* — the Tracy protocol renamed (D10), plus a socket in the frame loop. A tailed file answers the same question at zero engine cost.
- *Remote arm/disarm* — **already served**: a network handler calls `world.enable::<ProfilingScope>(e)` like any other system. The engine supplies the switch; the game supplies the wire.

### D27 — A game's handles live in ECS storage; the profiler does not store them

`DynZoneHandle` is **16 B, `Copy`, `Send + Sync`, with no thread affinity**, so a game stores it in a component or a `Resource`-owned column and emits from any lane. The profiler keeps the *descriptor*; the game keeps the *handle*. This is Principle 0 applied to the game's side of the seam: the durable per-entity association is ECS data, in ECS storage, owned by the game.

`Arena` / `ComponentPool` / `UnitId` remain **untouched by the profiler itself**, deliberately: it stores no per-entity data, so two-level addressing is not involved, and routing transport through `ComponentPool` would put a growth path on the emission side.

### D28 — What the extension asked for and this design refuses

| Asked | Refused | Reason |
|---|---|---|
| Toggle projected by an `IsEnabled` **observer** | yes | The kernel fires no observer on an enable-bit toggle (`enable_tag_api.rs:77-88`). Replaced by the drain-step projection (D20), which is cheaper than a system and honest about its ≤ 320 ns |
| A `ProfilerSet::{Retire, Read}` pair with an ordering edge | yes | The retire and drain run outside the schedule entirely (D16/D4a), so no edge is needed. Adding systems to the subsystem that measures scheduling perturbs its own product (F14) |
| 1-in-N sampling **at the call site** | yes | A per-site RMW on a shared line. Decimation happens at retention and via the scope bit. A game wanting 1-in-N writes it in its own code, visibly (X16) |
| Same-frame counter readback as a message bus | yes | X14 above |
| A second sink thread for telemetry | yes | 0.36 % of one frame in 120 does not justify a thread (X25) |
| A second `ARM_MASK` word (128 scopes) | deferred | 32 game bits are not yet full; a second word costs the hot path. Refuse until a title exhausts 32 |
| A live network viewer | yes | D26 |

---

## Statistics discipline (new in rev 3 — the VG R3 P4-6 lesson, made structural)

A game-facing profiler will hit every one of these constantly, so they are properties of the API, not advice in a paragraph.

| # | Rule | Enforced by |
|---|---|---|
| **S1** | **A band is `max(floor, twin, se_floor, quantum)`; no single term is the band.** | `resolve`'s signature takes both a `Floor` and a `Twin`; neither alone constructs a verdict (D11) |
| **S2** | **A zero control whose expected value is exactly zero measures DRIFT, not RESOLUTION.** P4-6's A0/A1 were the same configuration on a serialized deterministic GPU; the twin was 0 on all ten passes and the rule silently became "is nonzero", reporting a false RESOLVED. | The `se_floor` term is mandatory and is computed from the propagated SE of every median a reading is built from — `SE(median) ≈ 1.2533·σ/√n` (`vg_occ_split_timing.rs:315`). A twin of 0 can never shrink the band below it |
| **S3** | **The instrument's quantum is measured per sitting, never hard-coded**, and `timestampPeriod` is not it. | `measured_quantum_ns` in the reporter (`vg_occ_split_timing.rs:871-892`); the plan contains no numeric GPU quantum |
| **S4** | **Every reduced window has an ODD sample count**, so its median is an actual sample and sits on the lattice. | `WINDOW = 121`; `debug_assert!(WINDOW % 2 == 1)`; means are excluded from the quantum GCD |
| **S5** | **Never compose reduced statistics.** `median(a) + median(b) ≠ median(a+b)` — crossed by 144-240 ns in P4-6. | No reporter API adds two reduced values. Partition sums are formed per frame in the frame-major row, then reduced (D7) |
| **S6** | **Two adjacent stamps cannot establish an order**, and **equal timestamps cannot license a record-order conclusion**. | `__gpu_null` deleted (D5); record order is witnessed host-side by `RecordCensus::first_pair_of` (D17) |
| **S7** | **A number whose own resolution is unknown is not printed.** | Quantum `UNKNOWN` ⇒ every GPU number in that report is `NOT RESOLVED` (D11a) |
| **S8** | **An incomplete window produces no verdict.** | `NotResolvedReason::{WindowIncomplete, EpochBreak, LabelNotMeasured}` (D11) |

---

## Data structures

```rust
// ══════════════ boyko_utils::profiling_abi ══════════════
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

/// THE record. 16 B, 4 per cache line, one shape for every kind.
///
/// `begin` is ABSOLUTE TSC, not frame-relative: a frame-relative u32 would need a
/// shared per-frame base (a coherence miss on every worker at frame start) and would
/// overflow on a >1.4 s frame — the hitch most worth recording. Absolute u64 is also
/// what makes frame attribution a merge (A2) and the overlap matrix epoch-free.
#[repr(C)]
pub struct Sample {
    begin: u64,   // Span: TSC at open. Counter/Gauge: the VALUE. Extension: high 32 bits of dur.
    dur:   u32,   // Span: TSC ticks (saturating + flag + an Extension sample). Else 0.
    zone:  u16,
    flags: u16,   // [0..1] kind (Span|Counter|Gauge|Extension) | [2] saturated
                  // | [3] gpu-origin | [4..15] reserved       (no depth field: D3a)
}
const _: () = assert!(size_of::<Sample>() == 16 && align_of::<Sample>() == 8);

/// Writer and reader halves on SEPARATE lines. All mutable state is atomic:
/// no UnsafeCell, no plain field mutated through `&'static` (rev 1 had both).
#[repr(C, align(64))]
struct RegionWriter {
    buf:      AtomicPtr<Sample>,  // published ONCE at first arm (Release), never nulled (D15)
    write:    AtomicU32,          // Relaxed read by the sole owner; Release store after the bytes
    overflow: AtomicU32,          // dropped samples; cleared by fetch_sub(observed) (D24a)
    _pad:     [u8; 48],
}
#[repr(C, align(64))] struct RegionReader { read: AtomicU32, _pad: [u8; 60] }
#[repr(C, align(64))] struct RegionLane   { w: RegionWriter, r: RegionReader }

/// FOUR distinct lines: engine writer / engine reader / user writer / user reader.
/// The engine/user split is a false-sharing fix as well as an isolation fix — a game's
/// `write` cursor never invalidates the engine's (D19).
#[repr(C, align(64))] struct ZoneLane { engine: RegionLane, user: RegionLane }
const _: () = assert!(size_of::<ZoneLane>() == 256);

pub const LANE_WORKERS:    usize = 64;      // mirrors boyko_threadpool::MAX_WORKERS
pub const LANE_DISPATCHER: usize = 64;
pub const LANE_HOST:       usize = 65;      // host thread OUTSIDE install (D2)
pub const LANE_SPARE_0:    usize = 66;      // 2 spare claimable
pub const LANE_COUNT:      usize = 68;
pub const LANE_UNCLAIMED:  u16   = u16::MAX;
pub const REGION_CAPACITY: u32   = 2048;    // per region; 32 KiB/region, 4.25 MiB total (D19)

static ARM_MASK: CachePadded<AtomicU64>;             // 0 == disarmed. Own line, read-mostly (D20)
static LANES:    [ZoneLane; LANE_COUNT];             // 17 KiB BSS
static REGISTRY: [AtomicPtr<ZoneDesc>; ZONE_ID_SPACE];
static DYN_DESCS: SyncCells<ZoneDesc, MAX_DYN_BUDGET>;   // static arena, never freed (A7)
static DYN_NAMES: SyncCells<u8, DYN_NAME_BYTES>;         // static arena, never freed
thread_local! { static PROFILER_LANE: Cell<u16> = const { Cell::new(LANE_UNCLAIMED) }; }

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

/// 80 B, align 8 — computed field by field (F22 recomputed rev 2's wrong 72).
#[repr(C)]
pub struct FrameRecord {
    frame: u32, state: FrameState, flags: u8, rounds: u16,          //  8
    fixed_steps: u16, epoch: u16, drops: u32,                       //  8   (D16: N is recorded)
    cpu_begin: u64, cpu_end: u64,                                   // 16
    run_gross: u64,                                                 //  8   __frame
    fixed_total: u64, main_total: u64,                              // 16
    instrument_measured: u64, instrument_estimated: u64,            // 16   split (F18)
    gpu_total: u64,                                                 //  8
    wall_ns: u64,                                                   //  8   labelled with its bound
}
const _: () = assert!(size_of::<FrameRecord>() == 88);   // 8+8+16+8+16+16+8+8 = 88

/// FRAME-MAJOR columns: index [frame * zone_stride + zone]  (D8, decided with numbers).
/// EVERY slice below is a sub-slice of ONE VmReservation owned here — no Box, no Vec (F7).
pub struct Profiler {
    vm: VmReservation,                 // the single backing reservation (memory/vm.rs:99)
    zone_stride: u32,                  // ENGINE_ZONE_SLOTS + armed_dyn_budget, fixed at arm

    total: &'static mut [u64],   // 1024*121*8 = 991 232 B = 968 KiB. Span: Σ ticks; C/G: level
    count: &'static mut [u16],   // 1024*121*2 = 247 808 B = 242 KiB
    min:   &'static mut [u32],   // 1024*121*4 = 495 616 B = 484 KiB
    max:   &'static mut [u32],   // 1024*121*4 = 495 616 B = 484 KiB
    label: &'static mut [u8],    // 1024*121*1 = 123 904 B = 121 KiB — MEASURED/NOT_BRACKETED/…
                                 // columns total = 19 B * Z * 121 = 2 299 KiB = 2.25 MiB

    lifetime: &'static mut [LifetimeAcc],  // [Z] tier B: 24 B each = 24 KiB       (D22)
    hist_of:  &'static mut [u8],           // [Z] zone -> hist slot, 0 = none      (D22)
    hists:    &'static mut [HistSlot],     // [cfg.hist_slots] 400 B each
    sys_of:   &'static mut [u16],          // [Z] zone -> system index, L1-resident (F19c)

    frames:    &'static mut [FrameRecord], // 121 * 88 = 10 648 B  = 10.4 KiB
    rounds:    &'static mut [RoundRecord], // 121 * 32 * 24        = 90.8 KiB   (analysis/Deep)
    legs:      &'static mut [LegSummary],  // 8 * 16 * 48          =  6.0 KiB   (analysis)
    frame_begin_tsc: &'static mut [u64],   // 121 * 8              =  0.95 KiB  — the A2 cut

    #[cfg(feature = "profiling-analysis")]
    compat:    &'static mut [u64],         // 1024^2 bits                     128 KiB
    #[cfg(feature = "profiling-analysis")]
    intervals: &'static mut [Interval],    // [8 * 2048]                      256 KiB

    scope_entity: [Entity; 64],        // the ECS source of truth for ARM_MASK (D20)
    scope_count:  u8,
    clock:   ClockCalibration,         // ticks_per_ns, calib_cv, calib_rejected, epoch
    quantum: [u64; 8],                 // per channel; GPU from measured_quantum_ns (D11a)
    cursor:  u32,
    drops:   DropCounters,             // 15 u64 classes, per region where applicable (D24b)
    telemetry: Option<TelemetryWriter>,
}
```

**Sizing (`Z = 1024`, `WINDOW = 121`), computed not asserted:**

| Configuration | Lanes | Columns | B/C tiers | Analysis | Frames + rounds + legs + cut | GPU host | **Total** |
|---|---|---|---|---|---|---|---|
| **Retail** (`Always`, analysis off, `Z = 256`, `hist_slots = 0`, `REGION_CAPACITY = 128`) | 272 KiB | 575 KiB | 6.8 KiB | — | 11.4 KiB (no rounds, no legs) | 8 KiB | **≈ 873 KiB = 0.85 MiB** |
| **Dev, armed, analysis off** (`Z = 1024`, 64 hist slots) | 4.25 MiB | 2.25 MiB | 49 KiB | — | 108 KiB | 8 KiB | **≈ 6.65 MiB** |
| **Dev, armed, analysis on** | 4.25 MiB | 2.25 MiB | 49 KiB | 384 KiB | 108 KiB | 8 KiB | **≈ 7.0 MiB** |
| **Dev, `dyn_budget = 3072` (`Z = 7168`)** | 4.25 MiB | 15.7 MiB | 344 KiB | 384 KiB | 108 KiB | 8 KiB | **≈ 20.8 MiB**, and `W9211` fires |

Retail reaches ≤ 1 MiB through three tier consts together — `REGION_CAPACITY = 128` (272 KiB of lanes, ample because `Always` mints no per-system and no per-pass zones), `ENGINE_ZONE_SLOTS = 256`, and `hist_slots = 0` — plus the `profiling-analysis` `#[cfg]` removing `compat`, `intervals`, `rounds` and `legs` entirely. **Allocated once at first arm, never freed** (D15). Before the first arm: 17 KiB `.bss` for `LANES` + the registry array + the dyn arenas (`DYN_DESCS` 3072 × 48 B = 144 KiB, `DYN_NAMES` 64 KiB — 208 KiB, zero-initialised, gated by G22).

```rust
// ══════════════ boyko_rhi_vulkan::present::gpu_zone ══════════════

pub struct GpuZoneRecorder {
    pools: [VulkanQueryPool; GPU_RING_DEPTH],
    slots: [FrameSlot; GPU_RING_DEPTH],
    next:  u32,
}
#[repr(C)]
struct FrameSlot {
    marks:   UnsafeCell<[u8; MAX_GPU_PAIRS]>,  // bit0 begun, bit1 ended — single producer (D5)
    zone_of: [ZoneId; MAX_GPU_PAIRS],          // pair -> ZoneId (boyko_utils::profiling_abi)
    seal:    AtomicU32,                        // THE release edge; == frame when marks are valid
    frame:   u32,
    submit_epoch: u64,                         // RenderEpoch at record time (D4a) — replaces
                                               // rev 2's sourceless `fence_seen: AtomicBool`
    used_pairs: u16,                           // bump allocator
    grace: u8,
    needs_cmd_reset: bool,                     // set when host reset is unavailable (D18)
}
pub const MAX_GPU_PAIRS: usize = 128;   // 256 queries — Bevy's QuerySet size
const _: () = assert!(MAX_GPU_PAIRS * 2 <= QUERY_POOL_WIDTH);
pub const GPU_RING_DEPTH: usize = 4;
pub const RETIRE_GRACE_FRAMES: u8 = 2;

const GPU_ZONE_QUERY_FLAGS: u32 = VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WITH_AVAILABILITY_BIT;
const _: () = assert!(GPU_ZONE_QUERY_FLAGS & VK_QUERY_RESULT_WAIT_BIT == 0);   // G2a's real red
```

---

## Public API

```rust
// ── emission (above the tier ceiling / feature off: expands to NOTHING) ──
declare_zone!(IDENT, name = "...", channel = ..., kind = ..., stage = ..., group = ...,
              scope = ..., tier = ...);
zone!(IDENT);                                 // RAII
#[must_use] zone_open!(IDENT) -> ZoneGuard;   // cross-function brackets
counter!(IDENT, value: u64);
gauge!(IDENT, value: u64);

// ── dynamic emission (game-defined zones; USER region, always) ──
pub struct ZoneSpec<'a> { pub name: &'a str, pub channel: Channel, pub kind: ZoneKind,
                          pub unit: Unit, pub scope: u8 }
pub fn register_zone(spec: ZoneSpec<'_>) -> Result<DynZoneHandle, RegisterError>;
zone_dyn!(handle);  counter_dyn!(handle, v);  gauge_dyn!(handle, v);
pub fn zone_dyn_open(h: DynZoneHandle) -> u64;      // FFI/script seam: returns an opaque token
pub fn zone_dyn_close(h: DynZoneHandle, token: u64);

// ── scopes (the ONLY runtime switch; no public mask setter) ──
#[derive(Component)] pub struct ProfilingScope { pub bit: u8, pub name: &'static str }
pub fn register_scope(name: &'static str) -> Result<u8, ScopeError>;   // #[cold], 32..63 for games
// arming/disarming a scope IS `world.enable::<ProfilingScope>(e)` / `.disable::<..>(e)` (D20)

// ── lanes (boyko_threadpool) ──
pub struct ProfilerLane(u16);
pub fn claim_profiler_lane() -> Option<ProfilerLane>;   // None when all spares are taken

// ── session control ──
pub struct ProfilerConfig {           // NOTE: `window` is NOT a field (F25) — WINDOW is a const
    pub scopes: u64,                  // initial ARM_MASK
    pub dyn_zone_budget: u16,         // 0..=3072; fixes zone_stride for the session
    pub hist_slots: u16,
    pub analysed_schedule: ScheduleLabel,   // which schedule's ConflictGraph is snapshotted (D9)
    pub telemetry: Option<TelemetryConfig>,
}
pub fn arm(world: &mut EcsMaster, cfg: ProfilerConfig) -> Result<(), ProfilerError>;
pub fn disarm(world: &mut EcsMaster);   // a mask store; frees nothing (D15)
// re-arm with a different geometry => E9213

// ── reading — kind-specific, so the wrong statistic is unreachable (D13) ──
impl Profiler {
    pub fn span(&self, id: ZoneId)    -> Option<SpanWindow<'_>>;
    pub fn counter(&self, id: ZoneId) -> Option<CounterWindow<'_>>;
    pub fn gauge(&self, id: ZoneId)   -> Option<GaugeWindow<'_>>;
    pub fn lifetime(&self, id: ZoneId)-> Option<LifetimeAcc>;      // tier B (D22)
    pub fn histogram(&self, id: ZoneId)-> Option<HistView<'_>>;    // tier C; quantiles as EDGES
    pub fn by_name(&self, name: &str) -> Option<ZoneId>;           // #[cold], setup/reporter only
    pub fn frame(&self, back: u32)    -> Option<&FrameRecord>;     // 0 = newest SEALED
    pub fn rounds(&self, back: u32)   -> &[RoundRecord];
    #[cfg(feature = "profiling-analysis")]
    pub fn concurrency(&self)         -> ConcurrencyReport<'_>;    // declared vs observed (D9)
    pub fn quantum(&self, ch: Channel)-> Quantum;                  // Known(u64) | Unknown (S7)
    pub fn drops(&self)               -> DropCounters;
    pub fn clock(&self)               -> ClockCalibration;
    pub fn tier(&self)                -> ZoneTier;
    pub fn epoch(&self)               -> u16;
    pub fn latency(&self)             -> LatencyTable;             // the printed table (D25)
}

pub struct SpanWindow<'a> { /* borrows the frame-major columns */ }
impl<'a> SpanWindow<'a> {
    pub fn median_frame_ticks(&self) -> Option<u64>;   // over per-frame TOTALS; n is ODD (S4)
    pub fn p95_frame_ticks(&self)    -> Option<(u64, u64, u64)>; // (p95, lo, hi) order-stat span
    pub fn mean_frame_ticks(&self)   -> Option<f64>;   // O(1), cached sum; EXCLUDED from S3's GCD
    pub fn per_sample_min_max(&self) -> Option<(u32, u32)>;      // distinct unit, distinct name
    pub fn halves(&self) -> (Option<u64>, Option<u64>);          // drift, always printed
    pub fn labels(&self) -> LabelCensus;
    pub fn n(&self) -> u32;
}
impl<'a> CounterWindow<'a> { pub fn rate_per_frame(&self) -> Option<f64>; pub fn level(&self) -> u64; }
impl<'a> GaugeWindow<'a>   { pub fn median(&self) -> Option<u64>; pub fn min_max(&self) -> Option<(u64,u64)>; }

// ── contrast: the ONLY way a delta leaves this system ──
pub struct Floor { /* rel, workload, sessions, repeats, path */ }
impl Floor { pub fn from_session_file(path: &Path) -> io::Result<Floor>; }   // the ONLY ctor
pub struct Twin { /* ticks, rounds, workload */ }
impl Twin  { pub fn from_zero_control(control: &LegSummary) -> Twin; }        // no sigma param

pub enum NotResolvedReason { BelowBand, FloorWorkloadMismatch, TwinWorkloadMismatch,
                             WindowIncomplete, EpochBreak, LabelNotMeasured }
pub enum Contrast {
    Resolved    { median_delta_ticks: i64, p10: i64, p90: i64, n: u32, band_ticks: u64,
                  floor_ticks: u64, twin_ticks: u64, se_floor_ticks: u64, quantum: Quantum,
                  order_bias_ticks: i64, control_cv: f32 },
    NotResolved { reason: NotResolvedReason, /* …the same fields, all populated… */ },
}
pub fn resolve(a: &LegSummary, b: &LegSummary, floor: &Floor, twin: &Twin) -> Contrast;

pub struct ContrastPlan { /* ABBA sequence + leg boundaries */ }
impl ContrastPlan {
    pub fn abba(rounds: u32, frames_per_leg: u32, zones: &[ZoneId]) -> Self;
    pub fn next_leg(&mut self) -> Option<Leg>;      // the CALLER applies the A/B configuration
    pub fn seal_leg(&mut self, p: &mut Profiler);   // folds the live window into a LegSummary
    pub fn summaries(&self) -> &[LegSummary];
}

// ── artifact + stream ──
pub fn append_artifact(p: &Profiler, path: &Path) -> io::Result<()>;   // #[cold], TOML, dev only

// ── diagnostics seam: the single site the logging plan re-points ──
pub(crate) fn emit_diag(code: DiagCode, fields: &[(&'static str, DiagValue)]);  // #[cold]
pub fn flush_on_panic();   // #[cold]; called BY the logging plan's single hook, never installed here

// ── RHI seam (three verbs; NONE of them can block — D4) ──
fn read_query_pool_pairs_available(&self, pool: &A::QueryPool, pair_count: u32,
    scratch: &mut [u64], out_begin_ticks: &mut [u64], out_dur_ticks: &mut [u64],
    out_available: &mut [u8]) -> Result<(), Self::Error>;
fn reset_query_pool_host(&self, pool: &A::QueryPool, first: u32, count: u32)
    -> Result<(), Self::Error>;
fn host_query_reset_supported(&self) -> bool;
```

**Deliberately absent:** any function returning a bare delta · any ns value without its `calib_cv` · any GPU reader that can block · any accessor that panics on the wrong `ZoneKind` · **any `Floor` constructor taking a sigma or a single sitting** · **any public `ARM_MASK` setter** · any `&str`-keyed emission · any point-estimate quantile from a histogram.

---

## Algorithms for critical paths

### A1 — `ZoneGuard::open` / `Drop`

```
open:  0. const { SITE_TIER <= GLOBAL_TIER }        -- compile-time; false => nothing is emitted
       1. ARM_MASK.load(Acquire) -> bt scope_bit; not taken -> NULL guard, return      (D1/F11)
       2. HANDLE.id.load(Relaxed); UNASSIGNED/RESERVED -> #[cold] register (D6)
       3. rdtsc
       4. guard = { begin, id, lane }        // lane = PROFILER_LANE.get(), ONE load (D2/F12)
drop:  5. id == DISABLED || lane == LANE_UNCLAIMED -> #[cold] count and return
       6. rdtsc; d = now - begin;  if d > u32::MAX -> #[cold] emit Extension sample, set flag
       7. reg = &LANES[lane].<REGION>;  buf = reg.w.buf.load(Acquire); idx = reg.w.write.load(Relaxed)
       8. idx - reg.r.read.load(Acquire) >= REGION_CAPACITY -> #[cold] overflow.fetch_add(1); return
       9. store 16 B at buf[idx & MASK]
      10. reg.w.write.store(idx + 1, Release)
```

`REGION` is a **compile-time constant** of the site (`Engine` for `declare_zone!`, `User` for `zone_dyn!`), so step 7 is an immediate offset, not a branch.

Complexity O(1); ~6 instructions + 2 `rdtsc` armed, 1 load + 1 predicted branch disarmed. Cache: a monotone cursor, 4 samples per line, ~0.25 misses/sample, write-allocated. **No non-temporal store** — the drain reads these bytes within one frame, so evicting them is strictly worse. **No software prefetch** — the hardware stride prefetcher already covers a monotone cursor. Branches: two, both `#[cold]`-biased. SIMD: none wanted (a 16 B store is one instruction).

**Why `buf` cannot be null at step 9** (F11): `arm()` stores the slab pointers `Release` **before** it stores `ARM_MASK` `Release`; step 1 is an `Acquire` load, so observing a set mask happens-after the pointer publication. A `debug_assert!(!buf.is_null())` records the invariant, and a loom case exercises it.

### A2 — Drain (top of `App::update_with_delta`, for CLOSED frames only)

Runs **before** step ① `Time::advance_with` (`crates/boyko_ecs/src/ecs/core/app/app.rs:655-676`) — the single funnel both `App::update` (`:736`) and the host's `app.update_with_delta(dt)` (`crates/boyko_app/src/runner.rs:1321`) pass through (F2).

```
0. scope projection (D20): for b in 0..scope_count:
       bits |= (world.is_enabled::<ProfilingScope>(scope_entity[b]) as u64) << b
   if bits != ARM_MASK.load(Relaxed) { ARM_MASK.store(bits, Release) }
1. if ARM_MASK == 0 { return }                       // the disarmed cost: 1 load + 1 branch
2. epoch check: if elapsed since last drain > MAX_PLAUSIBLE_FRAME_TICKS ->
       #[cold] discard the in-flight window; epoch += 1; epoch_breaks += 1; W9216; calibrate()
3. cut = frame_begin_tsc[current]        // samples at or after `cut` belong to the live frame
   for lane in 0..LANE_COUNT, for reg in [Engine, User]:
       w = reg.w.write.load(Acquire)     // publishes every sample byte below w
       r = reg.r.read.load(Relaxed)      // the dispatcher is the sole consumer
       for i in r..w:
           s = buf[i & MASK]
           if s.begin >= cut { stop this region }        // leave the live frame's samples
           f = merge_walk(frame_begin_tsc, s.begin)      // O(1) amortised: a region is TSC-monotone
           if f is older than the retained window { drops.late += 1; continue }
           match kind:
             Span    -> total[f*Z+z] += d; count += 1; min/max
                        if sys_of[z] != NONE -> intervals APPEND {s.begin, d, sys, occ}   (F19b)
             Counter -> total[f*Z+z] = s.begin (level)
             Gauge   -> gauge fold
           if hist_of[z] != 0 { hist_fold(z, d) }        // A9, tier C
       obs = reg.w.overflow.load(Relaxed); reg.w.overflow.fetch_sub(obs, Relaxed);   // NEVER store(0)
       drops.<region>_overflow += obs as u64
       reg.r.read.store(w, Release)
4. lifetime_fold(row = current frame row)   // ONE sequential pass, row still in L1d (D22 tier B)
5. if window boundary && telemetry.is_some() { __telemetry_write() }                  // D23/A10
```

- Complexity O(samples); ~400/frame → 6.4 KiB read; 19 B × `Z` written per frame row (19 KiB at `Z = 1024`, D8/F15).
- Cache: region reads strictly sequential; column writes scattered **inside one row per column**. The lifetime pass and the histogram fold hit lines the sample loop just touched.
- Branching: one 3-way jump table on `kind`, the cut test, and a `hist_of[z] != 0` byte test.
- **Sealing:** a frame becomes `Sealed` when its drain completes **and** (`GpuPass` disarmed **or** its GPU slot retired). If neither holds after `GPU_RING_DEPTH + RETIRE_GRACE_FRAMES` frames it becomes `Partial`. So `frame(0)` is never permanently `None` with the GPU channel off.
- **Step 0's cost is ≤ 5 ns × `scope_count`** (`enable_tag_api.rs:100-105`) and is inside `__drain`, hence inside `instrument_measured` and outside `__frame` (D16).

### A3 — GPU slot retire (host-called, NOT a scheduled system)

Called by the runner between `wait_frame_in_flight()` and the record, on the line that already publishes `RenderEpoch` (`crates/boyko_app/src/runner.rs:1319`):

```
retire_gpu(world, recorder, render_epoch):
  for slot in ring where slot.in_flight:
    read_query_pool_pairs_available(...)          // never blocks; VK_NOT_READY -> Ok, bits clear
    if avail covers every bracketed pair: publish MEASURED; retire
    else if render_epoch >= slot.submit_epoch + FRAMES_IN_FLIGHT && slot.grace == 0:    // D4a
        marks = (slot.seal.load(Acquire) == slot.frame) ? read marks : all-zero
        per pair: (1,1,1)=>MEASURED (0,0,_)=>NOT_BRACKETED (1,0,_)=>TORN (1,1,0)=>LOST
        retire PARTIAL; drops.gpu_lost += lost_count            // COUNTED, not printed (F20)
    else: slot.grace -= 1
    on retire: reset_query_pool_host(..) if supported else set needs_cmd_reset (D18)
```

O(`GPU_RING_DEPTH` × pairs) = 512/frame. **Termination proof:** every slot retires either on availability or on the epoch deadline; `RenderEpoch` advances once per successful `vkQueueSubmit` (`frame_driver.rs:255-262`), and if it stops advancing the frame loop has stopped — which `flush_gpu` at teardown covers (D4a/F28). **No path waits on a query and no path waits on a fence.**

**Why NOT a `requires_dispatcher` system (F14).** `system_meta.rs:130-141` states that `requires_dispatcher` is set by `NonSendRes`/`NonSendResMut::init_access`, and *"Those params ALSO declare universal access (CR-B), so the existing `is_universal()` derivation resolves the system to `SystemKind::CpuExclusive`."* Pinning retire that way inserts **a full schedule serialisation point every frame** — in the subsystem whose headline product is a concurrency statistic, and against D16. Rev 2's M5 fold bought thread pinning at the price of the property D9 exists to measure. Running it at the host seam costs nothing, needs no `SystemSet`, and reads host state (the recorder, the epoch) where that state already lives. **Cost, stated:** the retire is a host-called function rather than an ECS system, which is less ECS-native in *shape*; the precedent is the adjacent line in the same loop, where the host publishes `RenderEpoch` into the world by hand.

### A4 — Window reduction, median, overlap (reporter, `#[cold]`)

- Reduction: strided gather per column, `WINDOW = 121` reads per zone per column; AVX2 8-wide over the gathered scratch.
- Median/p95: copy 121 values into stack scratch, sort, index. **`n` is odd, so the median is an actual sample and sits on the lattice** (S4). p95 at n=121 is the 115th order statistic — printed with its neighbours (`p95_lo`, `p95_hi`) so its rank uncertainty is on the page rather than implied.
- Partition sums: formed **per frame in the row**, then reduced (S5). There is no API that adds two reduced values.
- Quantum: `measured_quantum_ns` over every timestamp-derived value the sitting published, **means excluded** (S3/D11a).
- Overlap (analysis only): per compatible pair that both ran, interval intersection over the `intervals` ring — SoA `u64` compare, 4-wide. O(pairs that actually ran × `OVERLAP_FRAMES`), not O(S²) per frame.

### A5 — Leg sealing (contrast)

`ContrastPlan::seal_leg` folds the current window's ≤ 16 subscribed zones into a `LegSummary { zone, median, p95, n, labels, first_half, second_half, drops_in_window, epoch, workload: WorkloadTag }` (48 B) in the `legs` arena. `resolve` consumes **summaries**, never live windows (rev 1's `ZoneWindow` borrowed the live ring, so leg A's data was overwritten before leg B ended).

**Four fields are load-bearing, not decoration:** `drops_in_window != 0` ⇒ `WindowIncomplete`; a differing `epoch` ⇒ `EpochBreak`; a `workload` mismatch against the `Floor` or `Twin` ⇒ the corresponding mismatch reason; a non-`MEASURED` label ⇒ `LabelNotMeasured` (D11/F4/X8).

The A/B *configuration change* is applied by the caller (it must be — it is configuration); the plan owns only the sequence and the boundary signal.

### A6 — Floor session (offline, N processes)

`vg_decidability_floor.rs`'s protocol generalised: the **same** workload class in `SESSIONS = 7` separate processes, `REPEATS = 3` times, `FLOOR_SIGMA = 3.0 × CV` of the worst subscribed statistic, **all three repetition floors printed, never averaged**, written to `docs/PROFILING-FLOOR.md` together with the `WorkloadTag` they were measured on. `Floor::from_session_file` reads it and carries the tag, which `resolve` checks (F4).

### A7 — `register_zone` (dynamic, `#[cold]`, lock-free, allocator-free)

```
1. validate: spec.scope >= 32 else W9212, Err(EngineScopeRefused)
             spec.name.len() <= MAX_ZONE_NAME_BYTES else truncate + flag
2. id = DYN_ID_NEXT.fetch_add(1, Relaxed)
   if id >= armed_dyn_budget {
       DYN_ID_NEXT.fetch_sub(1, Relaxed);        // monotone bound restored; no id leaked
       drops.dyn_registrations_refused += 1; W9210; return Err(BudgetExhausted) }
3. off = DYN_NAME_NEXT.fetch_add(padded_len, Relaxed)
   if off + padded_len > DYN_NAME_BYTES { fetch_sub; W9210; return Err(NameArenaExhausted) }
4. copy name bytes into DYN_NAMES[off..]; build &'static str from the reserved range
5. write ZoneDesc into DYN_DESCS[id]              // sole writer: this thread's reserved slot
6. REGISTRY[ENGINE_ZONE_SLOTS + id].store(desc_ptr, Release)      // THE publication edge
7. return DynZoneHandle { id: ENGINE_ZONE_SLOTS + id, arm_bit: 1 << spec.scope }
```

O(name_len). No CAS loop, no spin, no allocation, no lock. **Steps 5→6 are the ordering contract:** the desc is fully written before the pointer is published, so any `Acquire` reader of `REGISTRY[i]` sees an initialised desc; and the handle — hence the ability to emit — is only returned after step 6, so a `Sample` can never carry an id whose desc is unpublished. The `fetch_add`/`fetch_sub` reservation can transiently over-report but never under-report, and every claimant re-checks the bound.

### A8 — Scope projection

Not an observer (there is none — `enable_tag_api.rs:77-88`) and not a system. It is **step 0 of A2**. ≤ 5 ns × `scope_count`, one `Release` store only when the value changes, `#[cold]`-free because it is already off the hot path. `ARM_MASK` toggling has no other public writer.

### A9 — `hist_fold` (drain inner loop, tier C)

```
z   = d.clamp(1 << 6, (1 << 30) - 1)      // two branchless clamps
e   = 63 - z.leading_zeros()              // lzcnt
m   = (z >> (e - 3)) & 7
idx = ((e - 6) << 3) | m                  // 0..191
slot.buckets[idx] = slot.buckets[idx].saturating_add(1)
slot.total += d as u64; slot.count += 1
```

~8 instructions, no branch except the saturating add's (`adc`/`cmov`-shaped). Executed only for zones with a slot. Measured target: `drain_cost` +18 % at 64 active slots. Saturation is counted (`hist_saturations`), never silent.

### A10 — Telemetry window write (`__telemetry_write`, dispatcher, `#[cold]`)

```
1. buf = encode[cur]                       // boot-allocated, double-buffered; 0 allocations
2. if first window in this file: write header (128 B)
3. for each zone whose id has not yet appeared in this file: append ZoneRow
4. for each subscribed zone: append WindowRec (40 B) from the reduced window
   fixed_elapsed_ns = FixedTime::elapsed()  (time/fixed_time.rs:162) — the determinism witness
5. file.write_all(&buf[..n])               // ONE syscall; NO cross-window buffering
6. bytes_written += n; if bytes_written > max_bytes { rotate }
7. cur ^= 1
```

O(subscribed zones). One `write_all` of ~1.6 KB (retail) or ~16 KB (dev) per 2 s. Measured target p95 ≤ 200 µs, **inside `instrument_measured`** (D16). Errors set `telemetry = None`, count `telemetry_write_errors`, emit `W9215` once — never panic, never retry in-frame.

---

## Multithreading model

| Datum | Sharing | Writer | Reader |
|---|---|---|---|
| `ARM_MASK` | shared, read-mostly, `CachePadded` | the drain's scope projection, `arm`/`disarm` | every emitter, **`Acquire`** |
| `LANES[n].<reg>.w.buf` | shared pointer | `arm` at **first arm only** (`Release`) | region owner (`Acquire`) |
| `LANES[n].<reg>` sample bytes | **single writer** = lane owner | lane owner | drain |
| `LANES[n].<reg>.w.write` | 1W/1R | lane owner (`Release`) | drain (`Acquire`) |
| `LANES[n].<reg>.r.read` | 1W/1R | drain (`Release`) | lane owner (`Acquire`) |
| `LANES[n].<reg>.w.overflow` | shared counter | lane owner (`fetch_add`, `Relaxed`) | drain (`fetch_sub(observed)`, `Relaxed`) |
| `REGISTRY[i]` | shared | first executor / `register_zone` (`Release`) | drain, reporter (`Acquire`) |
| `ZoneHandle.id` | shared | CAS `UNASSIGNED→RESERVED`, then store (`Release`) | emitters, **`Relaxed`** |
| `ENGINE_ID_NEXT` / `DYN_ID_NEXT` / `DYN_NAME_NEXT` | shared counters | any registrant (`fetch_add`/`fetch_sub`, `Relaxed`) | same |
| `DYN_DESCS[k]` / `DYN_NAMES[..]` | **single writer per reserved range** | the thread whose `fetch_add` reserved it | anyone, gated by `REGISTRY`'s `Release` |
| `PROFILER_LANE` | thread-local | the owning thread, once (D2) | the owning thread |
| `Profiler` | dispatcher/host-only for mutation | drain / retire / reporter | `Res<Profiler>` systems |
| `FrameSlot.marks` | 1W/1R | recorder (plain stores) | retire, gated by `seal` |
| `FrameSlot.seal` | 1W/1R | recorder (`Release`) | retire (`Acquire`) |
| telemetry encode buffers | dispatcher-only | dispatcher | `write_all` (OS buffer — named FFI exception) |

**Ordering, each justified.**

- **`ARM_MASK` `Acquire` load / `Release` store.** It gates the lane `buf` pointer, which is published before it. **On x86-64 an `Acquire` load of an aligned word is a plain `mov`** — zero extra instructions — so rev 2's "Relaxed because Acquire would cost a fence off x86" was backwards *and* missed the obligation (F11). The scope projection's store is `Release` for the same reason. A worker seeing a stale mask for one frame records or skips one frame's samples, which is not a correctness property (G12 asserts the *next* frame).
- **`w.buf` `Release`/`Acquire`** — the slab's initialisation must happen-before any write through the pointer; the only pointer-carrying edge on the transport side.
- **`w.write` `Release`/`Acquire`** — the sole publication edge for sample bytes, the same edge `EventBuffer::write_len` uses (`events/event_buffer.rs:79-81`).
- **`r.read` `Release`/`Acquire`** — publishes "these slots are reusable" before the producer may overwrite.
- **`overflow` `Relaxed` both sides** — a counter with no ordering obligation; the `fetch_sub(observed)` form is what makes a concurrent producer increment survive the clear (D24a).
- **`ZoneHandle.id` `Release` store / `Relaxed` load, `REGISTRY[i]` `Release`/`Acquire`** (F10, one specification only). The emitter's `Relaxed` id load is sound because **it never dereferences a desc** — it stores a bare `u16`. The desc edge is carried entirely by `REGISTRY[i]`: a drain that `Acquire`-loads the value the registrant `Release`-stored synchronises-with it, and every desc byte was written before that store (A7 step 5 → 6, D6 step 4 → 5). This holds whether or not the emitter is the registrant.
- **`DYN_*_NEXT` `Relaxed`** — monotone reservation counters carrying no data; the data edge is the `REGISTRY` store.
- **`FrameSlot.seal` `Release`/`Acquire`** — one edge for the whole mark array, which is why no 128-bit atomic is needed.
- **No `SeqCst` anywhere.**

**Data-race freedom.**

1. *Sample bytes.* Exactly one writer per **region** by construction (D2 + D19): workers write `LANES[worker_id]`, the dispatcher `LANES[64]`, the host thread its claimed lane, unclaimed threads nothing. Within a lane the region is a compile-time constant of the emitting site, so one thread's engine and user regions are two independent SPSC rings with the same producer. One OS thread holding two lane identities over its life is serial, so each region still has one writer. Producer touches `[write, read + CAP)`; consumer touches `[read, write)`; disjoint given A1 step 8. Textbook Lamport SPSC — no CAS, no ABA (monotone `u32` cursors, masked indexing).
2. *Cursor wrap* (F23, rev 2's "49 days" was 24× wrong). `u32` wraps after 4.295 G samples: at 400 samples/frame/60 Hz that is **≈ 49.7 hours**; at a hot game lane's 2000/frame it is **≈ 9.9 hours**. Both are reachable in a soak or a long session, so correctness across one wrap is required, not incidental — the masked-index + unsigned-difference form provides it, and a unit test drives the cursor across `u32::MAX`.
3. *Overflow-counter wrap.* Impossible between two drains by D24c's arithmetic; accumulated into `u64`.
4. *False sharing.* `ZoneLane` = 256 B with **four** distinct lines, pinned by `offset_of!` const-asserts; `ARM_MASK` `CachePadded`; lanes 256 B apart.
5. *Dynamic descriptor arenas.* Each slot has exactly one writer (the `fetch_add` reserver) and is never reused or freed, so no reader can observe a torn re-initialisation; every read path goes through the `REGISTRY` `Release`/`Acquire` edge. `SyncCells` carries a manual `unsafe impl Sync` with these two clauses spelled out.
6. *Store.* `Profiler` is mutated only outside the schedule (drain, retire, reporter — D16/A3), so every in-frame `Res<Profiler>` reader sees one consistent snapshot. **No new synchronisation primitive is introduced.**
7. *Scope projection.* The drain reads the world through `&mut EcsMaster` at a point where no system is running; the `ARM_MASK` store is the only cross-thread effect and carries no data.
8. *Teardown.* There is none (D15): the `VmReservation` is never released, `buf` is never nulled, `DYN_DESCS` slots are never reused. `is_in_system_run()` is used **only** as a same-thread setup assertion (`crates/boyko_threadpool/src/tls.rs:83` reads the calling thread's TLS), never as a cross-thread barrier.
9. *`Send`/`Sync`.* `ZoneLane`: manual `unsafe impl Sync`, three clauses adapted from `ThreadLaneWriter` (`events/event_buffer.rs:93-110`) — (a) single writer per region, enforced by D2's per-thread lane and D19's const region; (b) the consumer touches only `[read, write)`; (c) the atomics cover the cursors, and the sample bytes are covered by the `write` `Release`/`Acquire` edge — **not** by a `&mut` synchronisation point, because a `static` has none (this is why rev 2's `EventBuffer` analogy was withdrawn, F7). `FrameSlot`: `Sync` by single-producer + `seal`. `ZoneGuard` is `!Send` via `PhantomData<*const ()>` — it carries a lane index bound to the current thread. `DynZoneHandle` **is** `Send + Sync + Copy` — 16 B of plain data with no thread affinity, which is what lets a game store it in a component and emit from any lane.
10. *Panic.* `ZoneGuard::drop` runs during unwind, so a panicking system's zone closes. Moot in practice: `crates/boyko_threadpool/src/worker.rs:157-168` aborts on worker panic. `flush_on_panic()` is called by the logging plan's single process-global hook and touches only host-owned state.

**Partitioning.** CPU zones partition by lane **and** by region (no stealing, no redistribution, no contention). GPU zones partition by frame slot; exactly one thread touches a slot at a time. Reporter and telemetry writer are single-threaded and `#[cold]`. **Rev 3 adds no thread.**

---

## Integration

| File | Change |
|---|---|
| `crates/boyko_utils/Cargo.toml` | unchanged — still **zero dependencies** |
| `crates/boyko_utils/src/lib.rs` | `+ pub mod profiling_abi;` |
| `crates/boyko_utils/src/profiling_abi/` | **new module group**: `channel, scope, zone, dyn_registry, sample, lane, clock, macros, tier` (F1) |
| `crates/boyko_threadpool/Cargo.toml` | **`+ boyko-utils = { path = "../boyko_utils" }`** — the one new edge for the lane TLS (F1/F12) |
| `crates/boyko_threadpool/src/tls.rs` | `+ set_profiler_lane()`, `+ profiler_lane()`, `+ #[cfg(debug_assertions)] OPEN_DEPTH`; re-export `claim_profiler_lane` |
| `crates/boyko_threadpool/src/worker.rs` | set `PROFILER_LANE` on `worker_main` entry, beside `set_current_worker_id` (F12) |
| `crates/boyko_threadpool/src/thread_pool.rs` | set/restore `PROFILER_LANE = LANE_DISPATCHER` on `install` entry/exit |
| `crates/boyko_ecs/build.rs` | **new** — `cargo:rerun-if-env-changed=BOYKO_PROFILING_TIER`, `=BOYKO_PROFILING_REGION_CAPACITY`, `=BOYKO_PROFILING_DYN_CAP`; emits `BOYKO_BUILD_HASH` |
| `crates/boyko_ecs/src/ecs/core/profiling/` | **new module group**: `store, drain, lifetime, hist, contrast, floor, concurrency, telemetry, ecs_control, plugin` |
| `crates/boyko_ecs/src/ecs/core/system/system_meta.rs` | `+ pub(crate) zone: ZoneId` (u16, unconditional in **both** axes, offset 242); `+ const _: () = assert!(size_of::<SystemMeta>() == 256)` beside the test at `:421` |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | `zone!` around `run_unsafe` (`:1267`) and the dispatcher-inline path (`:1108`); `RoundRecord` after the `to_spawn` loop (`:1222`) — all `Deep` tier |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | mint each system's `ZoneId` at `try_build` **under `const GLOBAL_TIER >= Dev`**, refusing (not panicking) past the budget; snapshot the `ConflictGraph` at arm under `profiling-analysis` |
| `crates/boyko_ecs/src/ecs/core/app/app.rs` | drain call at the top of **`update_with_delta` (`:655`)**, before step ①; `__frame` / `__events` / `__fixed_step` / `__main_run` zones (D16/F2). `update` (`:736`) unchanged |
| `crates/boyko_rhi/src/device.rs` | `+` three verbs; mark `read_query_pool_ns`/`_ticks`/`_pairs_ns` **FROZEN — no new callers** |
| `crates/boyko_rhi_vulkan/Cargo.toml` | **`+ boyko_utils = { package = "boyko-utils", path = "../boyko_utils" }`** — the second new edge (F1), justified in-file by the `boyko_sdf_math` precedent at `:44-49` |
| `crates/boyko_rhi_vulkan/src/ffi.rs` | `+ VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x0000_0004`; `+ PfnVkResetQueryPool` |
| `crates/boyko_rhi_vulkan/src/device.rs` | enable `hostQueryReset` when advertised; load `vkResetQueryPool`; expose the capability |
| `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` | the three verb bodies, beside `fetch_query_pair_ticks` (`:1249`); `GPU_ZONE_QUERY_FLAGS` + its `const _` WAIT_BIT assert (G2a) |
| `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` | **new** — `GpuZoneRecorder`, slot ring, marks+seal, 2×2 label, `submit_epoch` |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | `TsWitness` → `GpuZoneWitness`; `write_zero_pair` + epilogue gap-fill deleted at the retirement rung; `RecordCensus` counters + `first_pair_of` at the `vkCmd*` sites (`:107-156`'s pattern) |
| `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | 4 + 1 brackets ported |
| `crates/boyko_rhi_vulkan/src/present/swapchain.rs` | `PresentModeConfig`; `:199` becomes a probed choice with a loud fallback |
| `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` | **retired at rung 7**, not before |
| `crates/boyko_render/src/profiling_bridge.rs` | **new** — `retire_gpu(world, recorder, epoch)`, a host-called function (**not** a system — F14) |
| `crates/boyko_app/src/runner.rs` | call `retire_gpu` beside the `RenderEpoch` publication (`:1319`); `flush_gpu` on the teardown path (`:261`); claim `LANE_HOST` at boot; harness bodies + statistics helpers deleted **at rung 7** |
| `crates/boyko_app/src/gpu_scene/mod.rs` | env arming → `ProfilerConfig`; the `vb_timing_for_frame`-shaped predicate becomes a scope test |
| `crates/boyko_app/src/profiling/{report,artifact,stream}.rs` | **new** — TOML artifact (analysis feature) + binary stream writer |
| `crates/boyko_ui/src/profiling_overlay.rs` | **new, rung 15** — reference overlay over `Res<Profiler>` + `ProfiledZone` |
| `crates/boyko_demo` | overlay wiring + a console command driving `world.enable::<ProfilingScope>(e)` (the game-facing acceptance path) |
| `tools/prof_decode` | **new** — the telemetry stream decoder; the only reader of the binary format |

**Two Cargo edges, both new, both downward, both in-house, both argued in §Context/F1.** Rev 2 listed **no** Cargo change at all, which is why three of its rungs could not compile (F1).

**`Arena` / `ComponentPool` / `UnitId`: untouched, deliberately.** The profiler stores no per-entity data. **A game's `DynZoneHandle`s, by contrast, DO live in ECS storage** (D27) — the profiler does not store them; the game does, in a component or a `Resource`-owned column.

**Diagnostic codes (block `92xx`).** `W9201` engine zone registry exhausted (**warning, not error** — C-III/F5) · `W9202` GPU pair budget exhausted · `W9203` region overflow / unclaimed drops · `E9204` profiler already bound to another world · `W9205` zones LOST this window (**once per window, with a count** — F20) · `W9206` contrast NOT RESOLVED · `W9207` invariant TSC absent · `W9208` engine registry ≥ 90 % · `W9209` late samples dropped · `W9210` dynamic zone budget or name arena exhausted · `W9211` drain working set exceeds L1d (`zone_stride` too large) · `W9212` `register_zone` refused an engine scope (< 32) · `E9213` re-arm with a different geometry · `W9214` telemetry path unwritable at boot · `W9215` telemetry write error, streaming disabled · `W9216` TSC epoch break, window discarded, clock recalibrated · `W9217` GPU slots abandoned at teardown. All go through the single `emit_diag` seam (today `eprintln!`), the one site the logging plan re-points; **the profiler never prints from an emission path or from a per-pair loop** — every drop class is an atomic counted at the site and reported at the window.

---

## Implementation plan — every rung compiles the workspace alone

**Additive first, subtractive once.** Rev 3 amends rev-2 rungs in place rather than adding re-do rungs, because nothing is built yet and an interim design deferred "for later" is the pattern this project has been corrected on.

| Rung | Content | Gate(s) landing with it | Compiles alone because |
|---|---|---|---|
| **1** | `boyko_utils::profiling_abi`: `ARM_MASK: AtomicU64`, two-region `ZoneLane`, `REGISTRY`, `Sample`, `ZoneTier` + `GLOBAL_TIER` + `boyko_ecs/build.rs`, macros, clock; `boyko_threadpool` → `boyko_utils` edge + `PROFILER_LANE` set at `worker_main` / `install` | **G1, G4, G7, G22**, SPSC unit + property tests, the loom SPSC case | purely additive; `boyko_utils` keeps zero deps (F27: rung 1 no longer commits green with nothing exercising it) |
| **2** | `boyko_ecs::…::profiling`: `VmReservation`-backed store with an arm-time `zone_stride`, `drain.rs` (two regions, `fetch_sub` clear, epoch check), `arm`/`disarm`, `ProfilerPlugin`, world-bind check | **G4 (extended), G21, G23** | additive |
| **3** | `SystemMeta.zone` + const-assert; tier-gated minting at `try_build` with **non-terminal** refusal; the four `App` zones (`__frame`/`__events`/`__fixed_step`/`__main_run`) at `update_with_delta`; `RoundRecord`; `compat` + `intervals` + `ConcurrencyReport` under `profiling-analysis` | **G8, G9, G11** | one field in tail padding; four zone sites |
| **4** | RHI seam: three verbs + Vulkan impls + `ffi.rs` constants + `GPU_ZONE_QUERY_FLAGS` const-assert + Mock defaults and their pinning tests. **No consumer.** | **G2a, G2c** | old readers untouched |
| **5** | `boyko_rhi_vulkan` → `boyko_utils` edge; `gpu_zone.rs` + `RecordCensus` (incl. `first_pair_of`); VB brackets ported. **Serial A/B against the old collector** (never both armed in one frame — F17) | **G2b, G5, G10** | both collectors exist; every existing test still compiles and passes |
| **6** | gbuffer + SV0 ported; the R0 harness reads the new channel while the old one still exists | G10 extended to those passes | additive |
| **7 (the single subtractive rung)** | Delete `gpu_timing.rs`, the runner harness bodies and the statistics helpers **and** migrate every consumer in the same commit — the measured list is below | the post-rung grep gate | one commit, workspace green before and after |
| **8** | floor session, `Floor`/`Twin`/`resolve` + `NotResolvedReason`, reporter, TOML artifact, present mode (**labelling only if `Immediate` is unsupported — D12**), counters at `vkCmd*` sites, optional `profiling-alloc` | **G3a, G3b, G6, G13** | additive |
| **9 (v1.1)** | `VK_EXT_calibrated_timestamps` + rejection sampler; `cpu_gpu_offset` becomes a number with `max_deviation_ns` | — | additive |
| **10** | `dyn_registry.rs`: `DYN_DESCS`/`DYN_NAMES` static arenas + `SyncCells`, partitioned counters, `register_zone`, `DynZoneHandle`, `zone_dyn!`/`counter_dyn!`/`gauge_dyn!`, `zone_dyn_open`/`close` | **G11 (dyn half), G17, G20** | purely additive; drain/store already index by `ZoneId` |
| **11** | `ecs_control.rs`: `ProfilingScope` component, `register_scope`, the **drain-step projection** (A8), `ProfiledZone`, the `latency()` table | **G12** | additive; the mask exists from rung 1 |
| **12** | `lifetime.rs` + `hist.rs`: tier-B accumulators (always on when armed) and tier-C histograms (opt-in) | **G16, G18** | additive; both fold at the end of an existing drain pass |
| **13** | `stream.rs` binary telemetry + header/`ZoneRow`/`WindowRec`, rotation, failure handling; `tools/prof_decode`; session identity + `fixed_elapsed_ns` | **G15, G9 (telemetry clause)** | additive; the decoder is a separate binary |
| **14** | The retail leg: a CI job with `BOYKO_PROFILING_TIER=always --no-default-features --features profiling`, retail sizing consts, the `profiling-analysis` `#[cfg]` split | **G14** | a build-configuration rung; the workspace must be green in **both** tiers |
| **15** | `boyko_ui/profiling_overlay.rs` + `boyko_demo` wiring + a console command calling `world.enable::<ProfilingScope>(e)` | **G19** | additive; the acceptance path for the whole game-facing half |

**Rung 7's consumer list, MEASURED this revision** (`rg` over `TimestampCollector|VbTimedPass|Sv0TimedPass|TimedPass|PASS_COUNT`), **13 files** — rev 2 listed 15 names and omitted three production sites (F16):

| File | What names it |
|---|---|
| `crates/boyko_app/src/gpu_scene/mod.rs` | collector construction + arming |
| `crates/boyko_app/src/occlusion_force.rs` | pass enum |
| `crates/boyko_app/src/runner.rs` | the two env-var harnesses + statistics helpers |
| `crates/boyko_app/tests/sv0_deferred_term_bench.rs` | reads stdout |
| `crates/boyko_app/tests/vb_bench_totality_gate.rs` | **retired** — its mechanism is gone; replaced by G2a/G2b |
| `crates/boyko_app/tests/vg_occ_split_timing.rs` | reads stdout; migrates to the artifact |
| `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` | **deleted** |
| `crates/boyko_rhi_vulkan/src/present/mod.rs` | **re-export at `:52-56`** — unlisted by rev 2 |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | recorder |
| `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | **`use` at `:21`; three public `Option<&'a …Collector>` fields at `:2631`, `:2645`, `:2655`** — unlisted by rev 2 |
| `crates/boyko_rhi_vulkan/src/swapchain.rs` | **re-export at `:14-16`** — unlisted by rev 2 |
| `crates/boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs` | migrates to zones |
| `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | migrates to zones |

**This list is a LOWER BOUND, and the rung does not rest on it.** The gate is mechanical: after rung 7, `rg 'TimestampCollector|VbTimedPass|Sv0TimedPass'` over `crates/` must return **zero matches**, and the workspace must be green with `--workspace --all-targets`. A list that is one file short fails that gate rather than shipping.

**Ordering constraints:** 4 before 5 (seam before consumer) · 5 and 6 before 7 (the serial A/B is what licenses the deletion) · 8 after 7 · 10 after 2 · 11 after 10 · 12 after 2 · 13 after 12 · 14 after 13 · 15 after 11. Rungs 10-15 do not block 3-9 and may land interleaved; each is independently green.

---

## Metrics and validation

### Gates — every one has a showable RED and an explicit "cannot claim"

The second-pass review found **seven gates that could be GREEN while their claim was FALSE**, plus one target row whose proof was a `debug_assert`. All eight are repaired below; the repairs are marked **[F-fix]**. *A gate that cannot fail is worse than no gate*, so two rev-2 gates are deleted outright rather than patched.

| # | Claim | Showable RED | **CANNOT claim** |
|---|---|---|---|
| **G1** **[F-fix]** | A site above the tier ceiling (or with the feature off) emits **zero instructions** | **Two-sided, token-level:** (a) under feature-off, `zone!(NEVER_DECLARED_IDENT)` must **compile** — proving the expansion never names its argument; a `{ let _ = &$IDENT; }` else-arm makes it a build failure. (b) under feature-on the *same* source must **fail** to compile (`trybuild`). (c) an object-symbol census over the feature-off test binary shows zero references to `profiling_abi::ZoneGuard`/`record` | It cannot claim the *frame* got faster. It proves the instruction is absent from the binary, nothing more. Rev 2's "the macro cannot name the recorder" proved a **different proposition** and would have passed a `{ let _ = &IDENT; }` expansion (F8) |
| **G2a** **[F-fix]** | **No blocking GPU reader can exist** | `const _: () = assert!(GPU_ZONE_QUERY_FLAGS & VK_QUERY_RESULT_WAIT_BIT == 0)` — adding the bit is a **compile error**. Second clause: a source gate asserts the set of files naming `vkGetQueryPoolResults` equals a pinned list, so a new blocking reader in a new file fails by existing | It cannot claim the *driver* never blocks internally, only that this code never asks it to. Rev 2's grep scope (`gpu_zone.rs`, `profiling/**`) structurally excluded `rhi_impl/device.rs`, where the body must live (F3), and its behavioural red would have been a **hang**, which is not a showable red here (`vb_bench_totality_gate.rs:44-53`) |
| **G2b** | Label positive control | An unbracketed pass yields `NOT_BRACKETED` **and** a bracketed pass in the same frame yields `MEASURED` with a non-zero duration. A stub labelling everything `NOT_BRACKETED` fails clause 2 | It cannot claim the duration is *correct*, only that it is non-zero and labelled |
| **G2c** **[F-fix]** | Availability truth control | Poll before the fence ⇒ `available == 0` for every pair; poll after ⇒ `1` for bracketed, `0` for never-written. Flip `WITH_AVAILABILITY_BIT` to a wrong value ⇒ availability words are not written, `scratch` retains stale bytes, gate fails | Rev 2's version was passable by setting `WAIT_BIT` (the poll would block and the leg would **hang**, not fail). That escape is closed by G2a's const-assert, not by this gate — stated so the two are not confused |
| **G3a** **[F-fix]** | A delta below the band cannot return `Resolved` | A/A contrast (same code both legs) ⇒ `NotResolved { BelowBand }`. Shrink the band to a quantum ⇒ `Resolved` appears ⇒ gate fails. **Now unescapable in production too:** `Floor` has exactly one constructor (`from_session_file`), `FLOOR_SIGMA` is a `const`, and `resolve` checks `floor.workload == a.workload` | It cannot claim the *floor file* was measured honestly — only that the API cannot manufacture one. Rev 2 exported `Floor::from_aa_control(control, sigma)`, so production could hand `resolve` a one-sitting, caller-sigma floor and G3a constrained only the floor **the test** built (F4) |
| **G3b** | `Resolved` positive control | Contrast between a calibrated spin of K and 3K ticks ⇒ `Resolved`, `median_delta` within tolerance of 2K. `fn resolve(..) -> NotResolved{..}` fails here | It cannot claim `resolve` is right on *real* workloads; it is a synthetic with a known answer |
| **G4** | Overflow observability | Fill a region past capacity ⇒ `overflow > 0`, the artifact names it, **and the `u64` accumulator strictly increased across two consecutive drains**. Replace `fetch_sub(observed)` with `store(0)` ⇒ a producer increment between load and clear is lost ⇒ the accumulator lags a known injected count | It cannot claim samples were *not* lost — it claims the loss is counted |
| **G5** **[F-fix]** | Command census, two-sided | Disarmed ⇒ `profiling_cmds == 0` and every sub-counter 0. Armed ⇒ **`timestamps == 2 × recorded_pairs` and `recorded_pairs == declared_bracket_count`**. Record one profiling command on the disarmed path ⇒ clause 1 fails; drop one real bracket ⇒ clause 2 fails | It cannot claim the *pixels* are unchanged (golden pins do that, secondarily) — and golden pins cannot claim the *commands* are unchanged, because `PINS.toml:3` is a BMP SHA-256. Rev 2's armed clause `timestamps >= 2` was satisfied **by the instrument's own `__gpu_null` probe alone**, so a recorder that dropped every real bracket passed |
| **G6** | Partition check | A `PartitionGroup` containing a `TopOfPipe` member refuses to sum: declare a TOP zone into `PartitionGroup::VbRun` ⇒ the reporter prints `sum = NOT_VALID` and the test asserts it. **Second clause (S5):** the reporter has no API that adds two reduced values — a test that tries to must fail to compile | It cannot claim a `BottomOfPipe`-only sum is *complete*; a pass nobody bracketed is `NOT_BRACKETED`, not missing |
| **G7** **[F-fix]** | Unclaimed refusal **and correct lane attribution** | (a) Emit from an unclaimed `std::thread` ⇒ `unclaimed_drops > 0` **and** no lane cursor moved; routing unclaimed threads to lane 0 fails clause (a). (b) **Positive control:** a zone emitted on worker `k` lands in `LANES[k]` and nowhere else — omitting the `PROFILER_LANE` store in `worker_main` makes every worker "unclaimed" and fails (b) | It cannot claim the *host* lane is claimed in every host configuration — a host that never calls `claim_profiler_lane` drops, which is (a)'s behaviour and is counted. Clause (b) is what would have caught rev 2's contradiction between A1 step 4 and D2 (F12) |
| **G8** **[F-fix]** | Concurrency computability | A two-system schedule with a known conflict and a known-compatible pair ⇒ `declared` matches the conflict graph and `observed` is non-zero. **Configuration pinned:** the gate builds a pool with ≥ 2 workers and both systems spin a calibrated ≥ 100 µs, so the overlap is not marginal. If the pool has < 2 workers the gate **SKIPS with a printed reason, and CI fails on any nonzero skip count**. Discard intervals at drain ⇒ `observed` unavailable ⇒ red | It cannot claim the *serialisation index* of a real frame is accurate: `intervals` is an 8-frame ring with `INTERVALS_PER_FRAME = 2048` and an `intervals_dropped` counter, and it covers **one** schedule (`analysed_schedule`), with the rest in `systems_unanalysed`. Rev 2's version could be flaky-red on a small pool and self-overwrote a `Fixed` system N−1 times per frame (F19) |
| **G9** **[F-fix]** | Instrument disclosure, with **magnitude** | (a) `instrument_measured > 0` when armed; (b) `run_net < run_gross`; (c) **`instrument_measured >= instrument_zone_count × __cpu_null_median`** — a constant stub of `1` fails this; (d) a frame carrying a telemetry write has strictly larger `instrument_measured` than **both** neighbours by more than the band | It cannot claim the profiler's *total* perturbation is known. `instrument_estimated` (`zone_count × zone_cost`) comes from a different binary and is **printed, never subtracted** (F18). Rev 2 had only (a) and (b), which a constant `1` satisfied |
| **G10** **[F-fix]** | The old and new GPU collectors agree well enough to license the deletion | **Serial, not simultaneous** (F17): K frames with only `VbTimestampCollector` armed, then K frames with only `GpuZoneRecorder` armed, in one process, ABBA-ordered. Verdict is `resolve`'s: per pass, `\|median_old − median_new\| <= band`, where `band = max(floor, twin, se_floor, measured quantum)` — **not "one quantum"**, which on this box is a tolerance of **0** and therefore unsatisfiable (F6). Plus `RecordCensus::first_pair_of` must be **identical** between the two legs — the record-order witness (S6/D17). RED: shift a bracket by one command ⇒ the census differs ⇒ red before any timing is consulted | It cannot claim the two collectors are *bit-equal*: they write different queries in different pools at different stream positions, and the P4-6 lesson is that timestamps cannot license record-order conclusions. **The census clause, not the timing clause, is what actually licenses the deletion**; the timing clause is a magnitude sanity check with a band |
| **G11** | A game cannot starve the engine — **id space** | Exhaust `dyn_zone_budget` until `W9210`, **then** mint a fresh engine `declare_zone!` and assert it succeeds with an id `< ENGINE_ZONE_SLOTS` and its samples land in the window. Allocate dyn ids from `ENGINE_ID_NEXT` ⇒ red. **Second clause (C-III):** exhausting the *engine* range must **not panic** — it must return `DISABLED`, bump `zones_refused` and emit `W9201` once | It cannot claim a game gets the zones it asked for — only that its refusal is counted and does not propagate |
| **G12** | Scope toggle round-trip, two-sided | With scopes A and B armed, `world.disable::<ProfilingScope>(a)` ⇒ **the next** frame has zero A samples **and** a non-zero count of B samples; re-enable ⇒ A returns. An implementation that clears the whole mask passes clause 1 and fails clause 2; one that writes the wrong bit fails clause 1 | It cannot claim the toggle is *instantaneous*. The projection runs at the next drain, so the gate asserts the **next** frame, never the same one (D20). Stated in the API doc |
| **G13** | `resolve` refuses an incomplete window | Force a region overflow inside leg A ⇒ `NotResolved { WindowIncomplete }` with the delta fields still populated. Remove the refusal ⇒ `Resolved` on a truncated window ⇒ red. Sibling clauses: differing `epoch` ⇒ `EpochBreak`; a `LOST` label ⇒ `LabelNotMeasured`; a foreign `WorkloadTag` ⇒ `FloorWorkloadMismatch` | It cannot claim a *complete* window is trustworthy — completeness is necessary, not sufficient |
| **G14** | The retail tier is two-sided | Build with `BOYKO_PROFILING_TIER=always`; an object-symbol census shows **no** reference to the recorder from a `Deep` site **and** an `Always` site in the same binary **does** reference it. A ceiling that folds everything passes clause 1 and fails clause 2 | It cannot claim "zero cost in a shipping game". A symbol census proves the code is absent; it does not prove the frame got faster, and this box's own floor (6.3 / 14.3 / 4.7 / 13.5 %) makes a frame-time claim of that size undecidable |
| **G15** | The stream survives a kill | 900 frames with telemetry, `process::abort` mid-window, decode ⇒ N complete windows, header + `ZoneRow`s parse, N equals the number of window boundaries crossed. Buffer across windows ⇒ the decoded count is short ⇒ red | It cannot claim no telemetry is ever lost — only that loss is bounded by **one window** under `abort`. Power loss, a driver hang, or a kill during `write_all` are uncovered and no in-process gate can cover them |
| **G16** | Histogram fidelity | 10⁵ synthetic durations from a known distribution ⇒ `quantile(0.99)`'s bucket **edges bracket** the sorted-oracle p99, and the reported count equals the fed count. An off-by-one in the bucket index ⇒ the oracle falls outside ⇒ red | It cannot claim histogram quantiles can resolve a contrast. 6.25 % bucket width is the same order as the measured floor, which is why `resolve` does not consume histograms |
| **G17** | Dynamic path cost, one sitting | `zone_cost` reports static-armed ≤ 12 ns, dyn-armed ≤ 14 ns, static-disarmed ≤ 2 ns, dyn-disarmed ≤ 3 ns, script ≤ 18 ns — **all legs interleaved in one process**. Implement `zone_dyn!` with a `REGISTRY[id]` dereference to recover the scope bit ⇒ the dyn-armed leg exceeds 14 ns ⇒ red. Control: if the static leg regresses, the sitting is invalid and **no** dyn claim is made | It cannot claim the dyn path is fast *in a game*. It measures an isolated loop with the handle in a register; a real `DynZoneHandle` may be a cold load from a component. The bench measures the path's **floor** |
| **G18** | Lifetime accumulator agrees with the ring | Over 10 000 frames, `lifetime[z].count` equals Σ per-frame `count[z]` and `lifetime[z].max` equals max per-frame `max[z]`. Fold the lifetime row from the *previous* frame's row (after the ring overwrote it) ⇒ counts diverge ⇒ red | It cannot claim the accumulator is correct across an **epoch break** — a break discards the in-flight window by design, and the gate runs without one. A separate clause of G21 covers that |
| **G19** | The overlay read path is allocation-free | The reference overlay runs 600 frames under the counting-allocator gate ⇒ 0 allocations, **and** a control system that formats a `String` in the same test ⇒ > 0. Remove the control ⇒ the gate cannot distinguish "no allocations" from "the hook is not installed" | It cannot claim a *game's* overlay is allocation-free — only the reference one |
| **G20** | **A runaway game scope drops ZERO engine samples** | A spin loop emits `USER` zones until `user_overflow > 0`, while the engine emits a known number of `ENGINE` zones in the same frames ⇒ **every** engine sample is accounted for and `engine_overflow == 0`. Collapse the two regions into one ring ⇒ engine samples are lost ⇒ red | It cannot claim isolation under an **unclaimed** thread: a thread with no lane is refused entirely (G7) so it cannot overflow anything, and a mod spawning 100 threads exhausts the spares and has its zones refused and counted — different behaviour, separately gated |
| **G21** | TSC epoch break | Inject a synthetic forward jump of 10 s into the drain's clock source ⇒ `epoch_breaks == 1`, the in-flight window is discarded, `W9216` is emitted, the *next* window is complete, and no `FrameRecord` carries a duration above `MAX_PLAUSIBLE_FRAME_TICKS`. Remove the detector ⇒ a 10 s interval appears in `max` and in p95 ⇒ red | It cannot claim a **backward** TSC jump is handled the same way; a backward jump makes `dur` wrap and is caught by the `dur > u32::MAX` extension path, counted separately |
| **G22** | The dyn arenas are zero-initialised BSS | `llvm-readobj --sections` / `objdump -h` over the test binary shows the section owning `DYN_DESCS`/`DYN_NAMES`/`LANES` carries a size with no raw data. Initialise one element non-zero ⇒ the section grows raw data ⇒ red | It cannot claim BSS residency is *guaranteed*: PE/COFF `.bss` placement is a toolchain behaviour, not a language guarantee. The gate pins today's toolchain |
| **G23** **[F-fix]** | Resident memory is bounded **and allocated once** | In a test binary: `arm()` under the counting allocator, assert `std_bytes + vm.reserved_bytes()` ≤ the tier's target **and** > 0 (two-sided — a stub that allocates nothing fails); then a **second** `arm()` after `disarm()` allocates **0** additional bytes (D15). Rev 2's proof was a `debug_assert` (gone in release) plus an artifact field the same code computed — nothing observed the allocator | It cannot claim the *steady-state* footprint of a shipped title, only the boot total for a given `ProfilerConfig`. It also cannot see driver-side query-pool memory, which is reported separately from `DeviceCaps` |

**Deleted rather than patched:**

- **rev 2's `__gpu_null` quantum probe and every gate clause that used it** — measured-inert on this box (F6/D5).
- **rev 2's `vb_bench_totality_gate.rs`** — its mechanism (the totality epilogue) is deleted at rung 7, so the gate would pass vacuously. Replaced by G2a + G2b.

**No gate proves the profiler is honest about its own perturbation.** `instrument_estimated` is an estimate; only `__drain`, `__report`, `__hist_fold`, `__telemetry_write` and `__cpu_null` are measured directly. That sentence is written in the artifact, next to the number.

### Unit tests (assigned to rungs — F27)

**Rung 1:** SPSC ring empty/full/wrap **per region** · `u32` cursor driven across `u32::MAX` · `Sample` layout and flag round-trip · `ZoneLane` = 256 B with four distinct lines (`offset_of!` const-asserts) · concurrent first-execution mints **one dense id** across 16 threads with **no leaked counter value** · registry exhaustion is **non-terminal** and `zones_refused` increments · the 90 % warning fires exactly once · `dur` saturation emits an `Extension` sample carrying the high bits · `ZoneGuard` is `!Send` (compile-fail) · `zone!` feature-off accepts an undeclared identifier (G1a) and feature-on rejects it (G1b).

**Rung 2:** `SystemMeta` = 256 B in **both** tiers (const-assert + the test at `system_meta.rs:421`) · frame attribution — a sample straddling a boundary lands in the frame containing its `begin` · a sample older than the window increments `late` · sealing with `GpuPass` disarmed · `WINDOW % 2 == 1` · `zone_stride` arithmetic at `dyn_budget ∈ {0, 1, 256, MAX}` and the `W9211` threshold · `arm` twice with a different geometry ⇒ `E9213` · the `fetch_sub(observed)` clear survives a concurrent increment.

**Rung 3:** tier folding — a `Deep` zone's `ZoneId` is never minted at `GLOBAL_TIER = Always` · `FrameRecord.fixed_steps` equals the substep count for a 0-, 1- and 3-substep frame · `__frame` excludes `__drain`.

**Rung 4-6:** the 2×2 label truth table, all four rows · `VK_NOT_READY` maps to `Ok` with clear availability (Mock) · a slot retires on the epoch deadline with an unwritten pair · `flush_gpu` at teardown labels every in-flight pair and bumps `gpu_slots_abandoned` · `RecordCensus::first_pair_of` records the recording order, not the timestamp order.

**Rung 8:** `resolve` is `NotResolved` at exact equality with the band · every `NotResolvedReason` round-trips into the artifact · ABBA leg order is `A B B A` · leg summaries survive a window wrap · `Floor` cannot be constructed from anything but a session file (compile-fail) · a `Floor` with a foreign `WorkloadTag` ⇒ `FloorWorkloadMismatch` · `counter(id)` returns `None` for a `Span` zone (no panic) · `measured_quantum_ns` excludes means and returns `Unknown` on an all-zero sitting.

**Rungs 10-13:** `register_zone` refuses `scope < 32` with `W9212` · refuses past budget **without leaking an id** · a truncated name sets the flag · 16 threads registering concurrently produce 16 distinct ids, name ranges and `REGISTRY` entries · `DynZoneHandle` is `Send + Sync + Copy`, `size_of == 16` · the scope projection sets exactly one bit and clears exactly one · `scope_by_name` returns `None` for an unregistered name and never allocates · `hist_fold` bucket index at the clamp boundaries · `HistSlot` saturation increments `hist_saturations` exactly once per saturating add · lifetime `min` on an empty zone stays `u32::MAX` and reports "no samples", never a value · the stream header round-trips · a `ZoneRow` is emitted exactly once per zone per file, including after rotation · an epoch break discards the window and bumps `epoch`.

### Property tests

For any interleaving of `n` pushes and `m` drains: `pushed == drained + in_ring + overflowed`, **per region independently** · median/p95 match a sorted oracle over random windows · the overlap matrix is symmetric and reflexive · a `BottomOfPipe`-only partition group's per-frame member sum equals its run bracket exactly (the sum is formed per frame, S5) · frame attribution is a total function (every drained sample lands in exactly one frame or one drop counter) · **`engine_overflow > 0` implies the engine region alone exceeded its capacity** — user traffic never contributes (the formal statement of G20) · for any multiset of durations the histogram's `count` equals the number folded and every reported quantile's edges bracket the oracle · for any sequence of scope toggles, `ARM_MASK` equals the bitwise OR of the enabled scopes' bits and nothing else · for any `dyn_budget` and registration sequence, every returned `ZoneId` is in `[ENGINE_ZONE_SLOTS, ENGINE_ZONE_SLOTS + budget)` and all are distinct · decoding then re-encoding a stream is byte-identical.

### Loom / Miri

**Loom** (debug only — release loom binaries crash at startup on this box): one lane, **both regions**, 1P/1C each, capacity 2, 4 ops — no lost sample, no double-drain, no read of an unpublished slot · **the `arm` publication order** (`buf` `Release` before `ARM_MASK` `Release`, emitter `Acquire`-loads the mask): the emitter must never observe a set mask with a null `buf` (F11) · `register_zone` racing an `Acquire` read of `REGISTRY[id]` · the `seal`/`marks` publish · the scope projection's store racing an emitter's load (asserting the emitter sees one of the two values, never a torn one).

**Miri under Tree Borrows:** `unsafe impl Sync for ZoneLane`, the raw sample write through the published pointer, `FrameSlot.marks` `UnsafeCell` access, and `SyncCells` writes into `DYN_DESCS`/`DYN_NAMES` plus the `&'static str` construction from a reserved byte range — the one genuinely new `unsafe` surface.

### Benchmarks (criterion, `harness = false`)

`zone_cost` — **six legs, one sitting**: static-on / static-off-mask / static-off-tier / dyn-on / dyn-off-mask / script-FFI. The static-on leg gates at +25 % over the committed baseline; the dyn legs gate against their own baselines **and** against the static leg measured in the same sitting (a machine-wide regression must not be attributed to the dyn path).
`drain_cost` — four legs: 400 samples at `zone_stride` 1024 / 1280 / 4096, and 400 samples with 64 histogram slots active.
`scope_scan` — the drain's step-0 projection at 1 / 16 / 64 registered scopes (D20's ≤ 5 ns × `scope_count` claim).
`window_reduce` (1024 zones × 121) · `overlap_pairs` · `gpu_zone_retire` · `stream_encode` (400 `WindowRec`s + the `write_all`, p95 reported).

Protocol per `docs/BENCHMARKING.md`: median-of-N with an **odd** N (S4), High priority, all-core affinity, **never two bench jobs concurrently** (hard project rule; `target/` once reached 74 GB and took the disk to zero, masquerading as mingw errors).

**Naming:** no binary may contain `time` / `update` / `setup` / `install` / `patch` (Windows os-error-740). Hence `zone_cost`, `drain_cost`, `scope_scan`, `gpu_zone_retire`, `contrast_floor`, `stream_encode`.

### `debug_assert!` invariants

`lane < LANE_COUNT` · `zone < zone_stride` · `!buf.is_null()` at A1 step 9 · `OPEN_DEPTH == 0` at drain · `write - read <= REGION_CAPACITY` per region · `observed <= REGION_CAPACITY` at the overflow clear · `used_pairs <= MAX_GPU_PAIRS` · `pool.count == 2 * MAX_GPU_PAIRS` at reset (the width guard `gpu_timing.rs:492` already carries) · `!is_in_system_run()` at **arm** (same-thread assertion only; **not** at a scope toggle, which is system-callable by design) · kind matches the accessor · `frames[i].state != Pending` before the reporter reads · `WINDOW % 2 == 1` · `spec.scope >= 32` in `register_zone` · `region == User` for every dynamic emission.

**Release-live** (the GPU path inherits the driver's release profile, `crates/boyko_app/src/gpu_scene/mod.rs:7498`): the label computation, the retire deadline, **every one of the 15 drop counters**, the census clauses, the `NOT RESOLVED` verdict, the dyn-budget refusal, the epoch-break detector, the telemetry error path, and the histogram saturation counter. **A reporting obligation that vanishes in release is the vacuous-gate pattern by another route.**

---

## Answers to the review's open questions

1. **Which crate owns `Channel` / `GpuStage` / `PartitionGroup` / `ZoneId`, given `boyko_rhi_vulkan` may not depend upward?** `boyko_utils::profiling_abi` — the one existing crate below both `boyko_ecs` and `boyko_rhi_vulkan`, with zero dependencies and an existing precedent (`type_intern`). Two Cargo edges are added and both appear in the Integration table (F1).
2. **Where does the drain actually run, and which of `Fixed×N` / `Main` is the primary CPU number?** At the top of `App::update_with_delta` (`app.rs:655`), the single funnel. The primary number is `__frame` — the whole `update_with_delta` body after the drain — with `__fixed_step` (N per frame, N recorded) and `__main_run` as children (F2/D16).
3. **What is the non-blocking source of `fence_seen`, expressed as an RHI verb?** There is none, and none is added. It is **derived** from `RenderEpoch >= slot.submit_epoch + FRAMES_IN_FLIGHT`, the asset-retire rule stated by `frame_driver.rs:255-262` and already an ECS `Resource` at `asset_refcount.rs:55` (F13/D4a).
4. **With `__gpu_null` measured at 0, what is G10's tolerance and what does "quantum" protect?** `__gpu_null` is deleted. The quantum is `measured_quantum_ns` — the GCD of the sitting's timestamp-derived values, means excluded — and it is a **sub-floor** of the band, never the band. G10's tolerance is the full band, and its actual licensing clause is the **record-order census**, not the timing (F6/F17/D11a).
5. **If `MAX_SYSTEMS` rises to 1024, is 128 KiB + 256 KiB acceptable, and what is `MAX_ZONES` sized against?** Yes, behind `feature = "profiling-analysis"` (dev-only, off at retail). Zones are sized as `ENGINE_ZONE_SLOTS` (4096 dev / 256 retail) + a dynamic budget, per-system minting is tier-gated, and exhaustion is **non-terminal** so a default-on feature can never panic a legal app (F5/C-III).
6. **What binds a `Floor` to its workload, and what forbids a caller-chosen sigma?** A `WorkloadTag` carried by both the `Floor` and every `LegSummary`, checked by `resolve` (`FloorWorkloadMismatch`); and `FLOOR_SIGMA = 3.0` is a `const` with no parameter anywhere in the API. `Floor::from_aa_control` is deleted; the in-sitting control becomes a separate `Twin` type with a fixed reduction (F4/D11).
7. **128-pair witness representation:** a `[u8; MAX_GPU_PAIRS]` mark array in `UnsafeCell` plus a single `AtomicU32 seal`. One plain byte store per bracket; one `Release` store **per frame**, not per pass (D5).
8. **Which thread runs retire:** the host thread, at the runner seam beside the `RenderEpoch` publication — **not** a `requires_dispatcher` system, which `system_meta.rs:130-141` shows would resolve to `SystemKind::CpuExclusive` and serialise the schedule every frame (F14/A3).
9. **Is `profiling` default:** yes, feature default-on so shipped code carries the sites; `ARM_MASK == 0` by default, so the runtime cost is one predicted-not-taken branch and the drain is `if mask == 0 { return }`. The feature-off leg exists and is CI-built. The **tier** axis is orthogonal (default `Deep`, retail `Always`, own CI leg at rung 14). `SystemMeta.zone` is unconditional in both axes, so the 256 B pin is configuration-independent.
10. **Frame id of a sample:** not carried. Attribution is a merge walk of the region's TSC-monotone `begin` against `frame_begin_tsc[]`, stopping at the live frame's cut (A2). Late arrivals older than the window are counted (`W9209`).

## Answers to the scope extension's questions

1. **Static vs game-defined instrumentation** — D19/D27. Two authoring paths, one registry, one store, one drain. A Rust plugin crate uses `declare_zone!` verbatim and pays the engine's price; only *data-defined* zones take the dynamic path, at ≤ 14 ns (≤ 18 ns across FFI). The engine path is protected by a **partitioned id space** (G11) and a **partitioned ring region** (G20).
2. **Volume** — D22/D23/D24. The ring stays fixed and lossy because that is what buys 12 ns; retention is three tiers; the stream is window-granular at 2.9 MB/h retail; drops are counted per class **and per region** in `u64` with a non-wrap proof, and they now force `NotResolved`. Decimation happens at retention and via the scope bit, **never at the call site**.
3. **Runtime configurability** — D20. 64 scope bits; the **only** public input is the kernel's `IsEnabled` bit on a `ProfilingScope` entity, projected by **step 0 of the drain** — *not* by an observer, because `enable_tag_api.rs:77-88` documents that the enable path fires none. No public mask setter, no mirror, no dirty flag, no scheduled reconciliation system, no lock.
4. **Shipping builds** — D21. Three tiers, one `const` ceiling, retail = `Always` + `profiling-analysis` off ⇒ ≤ 1 MiB resident and a two-sided symbol gate (G14). Telemetry, crash diagnostics and a small counter set survive; per-system and per-pass zones, the contrast machinery, the concurrency analysis and the TOML writer do not.
5. **Consumption by the game** — D25. `Res<Profiler>` from any system, a **printed latency table** (CPU N−1, GPU N−4…N−2), `ProfiledZone` resolving ids once at setup, and a reference overlay at rung 15. Because the drain and the retire run outside the schedule, **no `SystemSet` and no ordering edge are needed**. The one refusal: the profiler is not an inter-system bus.
6. **Multi-process / replay / per-player** — D26. Session identity, build identity, an opaque player tag and replay correlation via `FixedTime::elapsed()` are **in**, at 44 B of header and 8 B per record. Cross-process aggregation and a live network viewer are **out**, with named re-entry conditions; remote arming is already served by (3).

---

## Open questions (remaining)

1. **`profiling-alloc` shim.** A global allocator is process-wide and perturbs everything it measures; the 19 zero-alloc gates answer "allocations per frame" more precisely, in test binaries, without perturbation. **Recommendation: build it, default off, `#[cfg]`-excluded at retail tier, artifact-labelled as a diagnostic mode whose numbers are not comparable to an unarmed run.**
2. **Artifact granularity.** The binary stream covers the "long capture" case, so the TOML can stay a document. Remains open only for a dev workflow that wants human-readable per-frame rows.
3. **v1.1 calibrated timestamps.** Deferred by D14. Two triggers now: a concrete cross-domain question ("is the CPU recording the frame, or waiting for the GPU to finish it?"), and an in-game overlay showing two axes, which will make users ask. Either must be answered with v1.1 or with a refusal, never with an uncalibrated offset.
4. **`Immediate` support on this box is unproven.** The design probes and records the resolved mode. If unsupported, rung 8's present-mode work reduces to labelling — **now stated in the rung table (D12), not only here.**
5. **Whether telemetry ever needs the log sink thread.** Decided against on the number (0.36 % of one frame in 120). Named trigger: `__telemetry_write` p95 > 200 µs on a real title, measured ⇒ hand off to `boyko_log`'s existing sink, **one thread for both subsystems, never two.**
6. **Scope namespace exhaustion.** 32 game-assignable bits. **Recommendation: refuse a second `ARM_MASK` word until a title actually exhausts 32**, because the second word costs the hot path and the first is not yet full.
7. **Histogram bucket geometry.** 3 mantissa bits / 6.25 % / 192 buckets / 400 B, chosen against the measured floor band (4.7-14.3 %). Widening to 4 bits is a config, not a redesign. Left open because no measured need exists.
8. **The in-tree lattice discrepancy.** `vg_occ_split_timing.rs:138` and `:881` say 16 ns; the odd-budget sitting measured 32 ns. This plan hard-codes neither and computes the quantum per sitting (S3). **Whether to repair those two prose sites is a separate, deliberately-scoped edit** — repairing doc-rot is as error-prone as writing it, and this plan is not the place.
9. **What the decoder becomes.** Rung 13 ships `prof_decode` as an in-tree CLI. Whether it eventually becomes a `boyko_ui` viewer inside the engine — the engine as its own Tracy, in-house, at zero dependency cost — needs no decision before rung 15.

---

## Checklist

Structure ✅ · Data structures ✅ (`repr`/align on every shared type; hot/cold split — `ZoneDesc` is `&'static` and never on the emission path, `DynZoneHandle` carries only what emission needs; **every size computed field-by-field and summed for four configurations**; false-sharing padding pinned by `const _` including the engine/user region split) · API ✅ (no `dyn`; no internal type in a signature; kind-typed windows; **one `Floor` constructor, no sigma parameter**; no bare-delta constructor; no panicking accessor; no public mask setter; no `&str`-keyed emission; no point-estimate quantile) · Multithreading ✅ (per-datum table; **every ordering justified exactly once**; no `SeqCst`; partition = lane × region; ten race-freedom clauses; `Send`/`Sync` incl. `ZoneGuard: !Send` and `DynZoneHandle: Send + Sync`; no teardown; **no new thread**) · Correctness ✅ (cursor wrap at both volumes with the corrected hours, overflow-counter non-wrap proof, `dur` saturation with a lossless extension, dense minting under contention with an executable order, dynamic minting with no leaked id, GPU deadline **from a real non-blocking source**, teardown flush, frame attribution, sealing, forgotten guard, panic unwind, multi-world, host-reset absence, TSC epoch break, telemetry write failure, name-arena exhaustion, histogram saturation, empty-window `min`) · Integration ✅ (**two Cargo edges, both named**; 25 files; 6 new module groups; `Arena`/`ComponentPool`/`UnitId` untouched with a reason; 15 rungs each compiling alone with the subtractive one isolated and its consumer list **measured, with a mechanical grep gate that does not rest on the list**) · Validation ✅ (**23 showable-RED gates, each with an explicit "cannot claim"**, eight of them repaired from green-while-false; two rev-2 gates deleted rather than patched; tests assigned to rungs; 11 property tests; loom incl. the `arm` publication order; Miri incl. the new `unsafe` surface; 8 benches with regression thresholds; a release-live list).

**N/A:** SIMD on the emission path — a 16 B store is one instruction; vectorisation belongs to A4 and is specified there. SIMD on `hist_fold` — a scatter into 192 buckets with data-dependent indices; gather/scatter would be slower than the 8-instruction scalar form on AVX2.

---

## What rev 3 changed that rev 2 had EARNED — stated explicitly, each argued

Nothing below is a silent regression. Each is a rev-2 property that a finding or the scope extension forced to move.

| Rev-2 property | Rev-3 state | Forced by | Argument |
|---|---|---|---|
| `WINDOW = 120` | **121** | S4 / P4-6 fact 7 | An even window makes every median the mean of two middle samples — a value no frame produced, half a lattice tick off. That is precisely how the 16 ns lattice was mis-derived |
| `MAX_SYSTEMS = 512` | **1024** | F5 | The kernel's own cap is `MAX_SYSTEMS_PER_SCHEDULE = 1024` (`schedule_builder.rs:70`). 512 silently truncated **both halves** of the headline concurrency statistic. Cost: `compat` 32 → 128 KiB, `intervals` 64 → 256 KiB, both behind `profiling-analysis` |
| Registry exhaustion **terminal** (`E9201`) | **non-terminal** (`W9201`, `DISABLED`, counted) | F5 / C-III | Per-system minting was unconditional on a default-on feature; a legal app with 1024 systems × 3 schedules would panic at build time. A missing *measurement* is not a wrong *answer*, so the `query_type_registry` precedent does not transfer |
| `resident ≤ 7 MiB`, proved by `debug_assert` | **four configurations**, ≈ 0.9 / 6.7 / 7.1 / 21 MiB, proved by a two-sided allocator + `VmReservation` gate | F26 / X5 | A `debug_assert` is gone in release and an artifact field is self-reported. G23 observes the allocator and asserts a **second** `arm` allocates 0 |
| `__gpu_null` as "the quantum probe" | **deleted** | F6 | Measured 0 on this box, every time. A measured-inert probe is not a probe, and keeping it made `resolve`'s `max(floor, quantum)` collapse to `floor` on the GPU channel |
| G10 tolerance "one quantum", simultaneous dual-record | **band-based, serial A/B, licensed by the record-order census** | F6 + F17 | The tolerance was 0 (unsatisfiable) and the configuration doubled the very command traffic D4 rejects the totality epilogue for. The census clause is the honest licence |
| Retire = a `requires_dispatcher` system | **a host-called function** at the runner seam | F14 | `requires_dispatcher` implies universal access ⇒ `SystemKind::CpuExclusive` ⇒ a full schedule serialisation point every frame, in the subsystem whose product is a concurrency statistic. Cost: less ECS-native in shape; precedent is the adjacent `RenderEpoch` line |
| Drain in `App::update` | **top of `App::update_with_delta`** | F2 | The windowed host calls `update_with_delta` directly (`runner.rs:1321`), so in the only configuration with a GPU channel the drain never ran |
| "the `Schedule::run` span" as the primary CPU number | **`__frame`, with `__fixed_step`×N and `__main_run` as children** | F2 | The frame is "Time → events → Fixed×N → Main" (`runner.rs:943`); that is not one interval |
| `Floor::from_aa_control(control, sigma)` | **deleted**; `Floor::from_session_file` only, plus a separate `Twin` | F4 | Two of rev 1's three refuted substitutions survived in it (one sitting, caller sigma), and no `Floor` was ever checked against the workload it licensed |
| `ARM_MASK` load `Relaxed` | **`Acquire`** | F11 | It gates the `buf` pointer. On x86-64 an `Acquire` load is the same `mov`, so the cost is zero instructions — rev 2's stated reason had the cost backwards |
| D2's three-branch per-emission lane resolution | **one TLS load**, set once per thread | F12 | The two specifications were incompatible and nothing set the TLS for workers, so every worker would have dropped |
| `instrument` as one number including an estimate | **`instrument_measured` + `instrument_estimated`**, only the former subtracted | F18 | Injecting a cross-binary median into a per-frame number, in the document that refuses to print unresolvable deltas |
| `CHANNEL_MASK: u32` | **`ARM_MASK: u64`** | X9/X10 | 64 scope bits; identical instruction count |
| `MAX_ZONES = 1024` fixed | **`zone_stride` fixed at arm**, tier-dependent engine slots + a dynamic budget | X2/X5 | A game must be able to declare its own zones; the store must be sized for them. Cost: one `imul` per sample at drain |
| Lane capacity 4096, one region | **2048 × two regions** | X4 | Region isolation is what makes G20's claim possible. Cost: engine burst headroom 4096 → 2048 samples = 5 frames, against a drain that runs every frame |
| `resolve` accepted any complete-looking window | **refuses drops and epoch breaks** | X8/C4 | This makes the engine side **stricter**: a bench that drops now produces no number instead of a wrong one |

---

## Findings disposition — rev 1 review (35 findings, carried forward)

| # | Finding | Disposition | Where |
|---|---|---|---|
| **B1** | `AtomicU128` does not exist | **FOLDED** — mark array `[u8; MAX_GPU_PAIRS]` + one `AtomicU32 seal`; one Release store per frame | D5, `FrameSlot` |
| **B2** | `WITH_AVAILABILITY_BIT` is `0x4`, not `0x20` | **FOLDED** — corrected against `ffi.rs:846,849`; G2c added | D4, G2c |
| **B3** | Floor: wrong instrument, wrong scope, wrong sigma | **FOLDED, then TIGHTENED in rev 3** — see F4 | D11, A6, G3a |
| **B4** | G5 vacuous — PINS are BMP SHA-256 | **FOLDED, then TIGHTENED in rev 3** — the armed clause is now an equality (F-gate-table) | D17, G5 |
| **B5** | No positive control for `Resolved` | **FOLDED** — G3b calibrated K vs 3K; also G2b | G3b, G2b |
| **B6** | `concurrency()` uncomputable from the store | **FOLDED, then CORRECTED in rev 3** — see F5, F19 | D9, G8 |
| **B7** | UAF on foreign lanes at disarm | **FOLDED** — no free ever; disarm is a mask store | D15 |
| **B8** | PR-5/PR-6 don't compile; test files unlisted | **FOLDED, then CORRECTED in rev 3** — see F1, F16 | Rung table |
| **M1** | Frame attribution / sealing undefined | **FOLDED** | A2 |
| **M2** | `depth` unincremented, UB in a `static` | **FOLDED** — debug-only TLS `OPEN_DEPTH` | D3a |
| **M3** | Two registries, no sync rule | **FOLDED, then MADE EXECUTABLE in rev 3** — see F9, F10 | D6 |
| **M4** | D2's premise false — the hazard is UNATTACHED→lane 0 | **FOLDED** | D2 |
| **M5** | Retire's thread specified twice | **FOLDED, then RE-DECIDED in rev 3** — see F14 | A3 |
| **M6** | Feature default unstated; drain uncosted | **FOLDED** | D1, target table |
| **M7** | The instrument sits inside its own primary number | **FOLDED, then RE-SITED in rev 3** — see F2 | D16 |
| **M8** | `WaveRecord.members` truncates above 256 systems | **FOLDED, then RE-BOUNDED in rev 3** — see F5 | D9 |
| **M9** | `rate_per_frame` panics | **FOLDED** — kind-specific windows | D13 |
| **M10** | `p10/p90` estimator undefined; control asymmetric | **FOLDED** | D11, `Contrast` |
| **M11** | No leg mechanism, no retention | **FOLDED** | A5 |
| **M12** | Layout fork asserted, not decided | **FOLDED, arithmetic CORRECTED in rev 3** — see F15 | D8 |
| **M13** | `calib_residual` = worst-of-N | **FOLDED** — `calib_cv` + `calib_rejected`; 20 ms window | D3 |
| **M14** | `hostQueryReset` fallback unspecified | **FOLDED** | D18 |
| **m1** | `total` sizing 4× off | **FOLDED** | `Profiler` |
| **m2** | `dur` saturation not recorded | **FOLDED** — `#[cold]` `Extension` sample | `Sample`, A1 |
| **m3** | Concurrent minting leaks ids | **FOLDED, then MADE EXECUTABLE in rev 3** — see F9 | D6 |
| **m4** | "no cold branch at all" is wrong | **FOLDED** | D6 |
| **m5** | 128 pairs exactly fills the witness | **FOLDED** — byte marks scale | D5 |
| **m6** | `min_max` vs `median`, one type | **FOLDED** | API |
| **m7** | p95 at n=120 is a 6th-order statistic | **FOLDED**; `n` is now 121 and odd (S4) | API, A4 |
| **m8** | `FrameRecord` undefined | **FOLDED, size CORRECTED in rev 3** — 88 B, computed field-by-field (F22) | Data structures |
| **m9** | `VK_NOT_READY` mapping unstated | **FOLDED** | D4 |
| **m10** | Artifact-at-exit loses partial evidence | **FOLDED** | API |
| **m11** | `Immediate` support assumed | **FOLDED**, and the rung's reduction is now stated in the rung table | D12 |
| **m12** | `SystemMeta` arithmetic 243 vs 244 | **FOLDED** — offset 242 / 244 total; `const _` assert | Invariant 2 |
| **m13** | Eight codes hard-wired to `eprintln!` | **FOLDED, then EXTENDED in rev 3** — `W9205` is once-per-window with a count (F20) | Integration |

## Findings disposition — rev 2 review (F1-F28)

| # | Severity | Finding | Disposition | Where |
|---|---|---|---|---|
| **F1** | BLOCKER | RHI crates cannot see `boyko_ecs`; rungs 4-6 unbuildable; no Cargo change listed | **FOLDED** — the emission ABI moves to `boyko_utils::profiling_abi` (zero-dep leaf, `type_intern` precedent); **two Cargo edges added and listed in the Integration table**. The opaque-`u16` alternative is named and rejected because it reintroduces `VbTimedPass`'s hand-maintained table | §Crate graph, Integration |
| **F2** | BLOCKER | Drain in `App::update`, which the windowed host never calls; "the `Schedule::run` span" is not one interval | **FOLDED** — drain at the top of `update_with_delta` (`app.rs:655`), the single funnel; the primary number becomes `__frame` with `__fixed_step`×N and `__main_run` as children, and `fixed_steps` is recorded per frame | D16, A2, Integration |
| **F3** | BLOCKER | "Never blocks" has no showable red at the site that can violate it | **FOLDED** — `WAIT_BIT` is made **unrepresentable**: `const _: () = assert!(GPU_ZONE_QUERY_FLAGS & WAIT_BIT == 0)`, a compile error. The source gate is kept but re-scoped to the set of files naming `vkGetQueryPoolResults` | D4, G2a |
| **F4** | BLOCKER | `Floor::from_aa_control` re-admits two refuted substitutions; no floor↔workload binding | **FOLDED** — constructor deleted; `Floor::from_session_file` is the only one; `FLOOR_SIGMA` is a `const`; a separate `Twin` type carries the in-sitting control with a **fixed** reduction; `WorkloadTag` on `Floor`, `Twin` and every `LegSummary`, checked by `resolve` | D11, A5, A6, G3a |
| **F5** | BLOCKER | `MAX_SYSTEMS = 512` vs the kernel's 1024; terminal exhaustion can panic a shipping build | **FOLDED** — `MAX_SYSTEMS = MAX_SYSTEMS_PER_SCHEDULE = 1024`; `systems_unanalysed` counter for other schedules; exhaustion **non-terminal**; per-system minting tier-gated; id space split engine/dynamic | D6, D9, C-III, G11 |
| **F6** | BLOCKER | G10's tolerance is 0 on this box; `__gpu_null` is measured-inert | **FOLDED** — `__gpu_null` deleted; the quantum is `measured_quantum_ns` computed per sitting; G10's tolerance is the full band and its licence is the record-order census | D5, D11a, G10, S3, S6 |
| **F7** | BLOCKER | Principle 0: `static` + leaked std-heap `Box`, on a refuted precedent | **FOLDED** — the `EventBuffer` analogy is **withdrawn** (`events/event_buffer.rs:202` is a field, not a `static`); the argument is remade on its own terms (the emitters have no world); and the backing memory becomes **one `VmReservation`** owned by the `Profiler` `Resource`, with the ABI leaf allocating nothing | §Principle 0, D8, `Profiler` |
| **F8** | BLOCKER | "Zero cost when off" asserted, not proved | **FOLDED** — G1 becomes two-sided and token-level: feature-off `zone!(UNDECLARED)` must compile, feature-on the same source must not, plus an object-symbol census. The cost (a typo'd `Deep` zone name is invisible at retail) is stated | G1, D1 |
| **F9** | MAJOR | D6's minting sequence is not executable (`n` used before it exists) | **FOLDED** — five-step total order over real values, with the refusal path restoring the counter | D6 |
| **F10** | MAJOR | `ZoneHandle.id` ordering specified twice, incompatibly | **FOLDED** — one specification: `id` store `Release` / load `Relaxed`; the desc edge is `REGISTRY[i]` `Release`/`Acquire`, with the argument restated on the registry slot and why the `Relaxed` id load is safe | D6, ordering table |
| **F11** | MAJOR | Arm publication order unspecified; hot path stores through `buf` with no null check | **FOLDED** — order pinned (slab → `buf` `Release` → `ARM_MASK` `Release`); the mask load becomes **`Acquire`** (zero instructions on x86-64); `debug_assert!(!buf.is_null())`; a loom case | D1, A1, Multithreading |
| **F12** | MAJOR | Lane resolution specified twice; nothing populates the worker TLS | **FOLDED** — one specification: `PROFILER_LANE` written once per thread at three named sites; emission is one load + one compare. G7 gains a positive control that a worker's samples land in its own lane | D2, A1, G7, Integration |
| **F13** | MAJOR | `fence_seen` has no source; `frame_driver.rs:265` mischaracterised | **FOLDED** — the anchor is corrected (`submission_epoch` is a submit counter), and `fence_seen` is **derived** from `RenderEpoch >= submit_epoch + FRAMES_IN_FLIGHT`, quoting `frame_driver.rs:255-262`. No new verb | D4a, A3, §Constraints |
| **F14** | MAJOR | `requires_dispatcher` ⇒ `CpuExclusive` ⇒ a schedule serialisation point | **FOLDED** — retire is **not** a system; it runs at the host seam beside the `RenderEpoch` publication. The cost (a host-called function, less ECS-native in shape) is stated with its precedent | A3, D25, Integration |
| **F15** | MAJOR | D8's line arithmetic wrong and omits a column | **FOLDED** — recomputed: 19 B/zone/frame ⇒ 304 lines / 19 KiB at `Z = 1024` (rev 2 said ≤ 256 / 16 KiB and omitted `label`). The conclusion survives at ~6.6×; the "fits L1d" claim is qualified with the drain's own 6.4 KiB | D8 |
| **F16** | MAJOR | Rung-7 consumer list omits three production files | **FOLDED** — the list is re-measured (13 files, with `present/mod.rs:52-56`, `scene_types.rs:21/2631/2645/2655` and `swapchain.rs:14-16` named), **and the rung is made not to rest on it**: the gate is a post-rung `rg` returning zero matches | Rung table |
| **F17** | MAJOR | Dual-record cross-check is confounded by its own instrumentation | **FOLDED** — dual-recording is replaced by a **serial** A/B (never both armed in one frame), and the licensing clause becomes the `RecordCensus` record-order equality rather than the timing | G10, rung 5 |
| **F18** | MAJOR | `instrument` mixes a per-frame measurement with a cross-binary median | **FOLDED** — split into `instrument_measured` (in-band) and `instrument_estimated` (with provenance); only the measured part is subtracted; `run_net` never contains the estimate | D16, `FrameRecord`, G9 |
| **F19** | MAJOR | G8's control unproducible; observed half self-overwrites; `sys` not derivable | **FOLDED** — (a) the pool/system configuration is pinned and a skip is a CI failure; (b) `intervals` becomes an **append** ring with `occ` and an `intervals_dropped` counter; (c) `ZoneDesc.system_index` + an arm-built `sys_of` side table | D9, A2, G8 |
| **F20** | MAJOR | `emit_diag` is `eprintln!`, per `LOST` pair, per frame | **FOLDED** — `LOST` is counted at the site and reported once per window with its count, the same rule as lane overflow | A3, D5, Integration |
| **F21** | MAJOR | `profile_spawn.rs` says ~20-30 ns **per pair**; the plan said 25 ns/call, 60 ns/pair | **FOLDED** — corrected to the measured text (`profile_spawn.rs:229-230`), and D1's rejection is restated on the corrected number, explicitly noting it is a 2× argument and no longer a 5× one | §Constraints, D1 |
| **F22** | MINOR | `FrameRecord` is 64 B, not 72 | **FOLDED** — the struct is redefined for D16's new fields and computed field-by-field at **88 B**, with a `const _` | `FrameRecord` |
| **F23** | MINOR | Cursor wrap is ≈ 49.7 hours, not 49 days | **FOLDED** — corrected, and stated for both volumes (49.7 h at 400/frame; 9.9 h at a game lane's 2000/frame), with a unit test driving the cursor across `u32::MAX` | Race clause 2 |
| **F24** | MINOR | Module path given three ways, all wrong | **FOLDED** — a path table in the header; the tree's actual `boyko_ecs::ecs::core::profiling` plus the new `boyko_utils::profiling_abi` | Header |
| **F25** | MINOR | `ProfilerConfig.window` vs `const WINDOW` | **FOLDED** — `window` is removed from the config; `WINDOW` is a tier-independent `const`; re-arm with a different geometry ⇒ `E9213` | API |
| **F26** | MINOR | Resident-memory proof is a `debug_assert` | **FOLDED** — G23: a two-sided boot-total gate over the counting allocator **and** `VmReservation::reserved_bytes()`, plus "a second `arm` allocates 0" | G23 |
| **F27** | MINOR | Rung 1 has no gate; tests are never assigned to rungs | **FOLDED** — every rung names its gates; unit tests are assigned to rungs | Rung table, Unit tests |
| **F28** | MINOR | When frames stop, neither retire horn fires | **FOLDED** — `flush_gpu` on the runner's teardown path (`runner.rs:261`) force-retires every slot as `Partial`, labels unavailable pairs `LOST`, counts `gpu_slots_abandoned` (`W9217`), release-live | D4a, A3 |

**Refuted / partially refuted findings — none silently.** Every `F` above is folded. Two carry a correction *to the review itself*, stated here rather than left implicit:

- **F3's premise about scope is right; its implied fix ("cover every file that can reach `vkGetQueryPoolResults`") is insufficient alone**, because a grep over a growing file set is itself a maintained list. The const-assert is the primary mechanism; the file-set gate is the backstop.
- **F7's `EventBuffer` citation is right and this plan adopts it, but the review's path (`event_buffer.rs`) omits the `events/` directory** — the file is `crates/boyko_ecs/src/ecs/core/events/event_buffer.rs`. All `event_buffer.rs` anchors in this revision carry the corrected path.

## Findings disposition — scope extension (X1-X25)

| # | Requirement / tension | Disposition | Where | Cost, stated |
|---|---|---|---|---|
| **X1** | Game-declared zones from a plugin crate | **RESOLVED WITHOUT A NEW PATH** — `declare_zone!` is exported from the leaf and re-exported through `boyko_ecs::prelude`; a Rust plugin pays the engine's 12 ns | D19 | none |
| **X2** | Game-declared zones from data / config / script / mods | **FOLDED** — dynamic registry over static desc/name arenas; `DynZoneHandle`; `zone_dyn!`; `zone_dyn_open/close` for FFI | D19, D27, A7, rung 10 | ≤ 14 ns (≤ 18 ns FFI), 208 KiB BSS, budget declared at arm, **not tier-foldable** |
| **X3** | Game must not degrade the engine — **id space** | **FOLDED** — partitioned counters, disjoint ranges, independent exhaustion codes | D6, D19, G11 | one extra atomic counter |
| **X4** | Game must not degrade the engine — **ring capacity** | **FOLDED** — two SPSC regions per lane; region is a compile-time const; separate slabs and counters | D19, `ZoneLane`, G20 | lane control 8.5 → 17 KiB BSS; engine burst headroom 4096 → 2048 samples (5 frames, argued) |
| **X5** | Store width vs "as many zones as the game wants" | **FOLDED** — arm-time `zone_stride`; budget default 256, cap 3072; `W9211` above L1d; a wide-stride bench leg | D8, G-bench | one `imul` per sample at drain; dev resident 6.7 → 21 MiB at the cap |
| **X6** | Hours of data vs a fixed lossy ring (**C-I**) | **FOLDED** — three retention tiers: the 121-frame ring + lifetime accumulators + opt-in log-linear histograms | D22, rung 12 | +24 KiB always, +25 KiB at 64 hist slots; **no per-frame history beyond ~2 s, ever** |
| **X7** | Drop count must stay honest at session scale | **FOLDED** — `fetch_sub(observed)` clear, `u64` accumulators, per-class **and per-region** attribution, a non-wrap proof, 15 release-live counters | D24, G4 | none |
| **X8** | A silently truncated capture is the vacuous-gate pattern | **FOLDED, AND IT TIGHTENS THE ENGINE SIDE** — any leg with a drop or an epoch break is `NotResolved { reason }` | D11, D24d, A5, G13 | **a bench that drops now produces no number instead of a wrong one** |
| **X9** | Runtime toggling without restart or a hot-path lock | **FOLDED, BUT THE PROPOSED MECHANISM IS REFUTED** — `ProfilingScope` + kernel `IsEnabled` stays; the **observer does not** (`enable_tag_api.rs:77-88`: "no hook / observer fire"). Projection is step 0 of the drain | D20, A8, G12, rung 11 | ≤ 5 ns × `scope_count` per frame (≤ 320 ns at 64 scopes), inside `instrument_measured`; toggle latency one frame |
| **X10** | Per-subsystem granularity | **FOLDED** — 64 scope bits: 0..7 channels, 8..31 engine, 32..63 game | D20 | 32 game bits; a hierarchy is refused to keep the gate one `bt` |
| **X11** | Shipping builds — what survives | **FOLDED** — `ZoneTier {Always,Dev,Deep}` + `const GLOBAL_TIER` via `option_env!` + `build.rs`; `profiling-analysis` split; retail ≤ 1 MiB; two-sided symbol gate | D21, G14, rung 14 | tier change rebuilds the workspace; ~12 KiB dead `.bss`; a `Deep` zone-name typo is invisible at retail |
| **X12** | Player telemetry / crash diagnostics / support log | **FOLDED** — append-only binary stream, window-granular, synchronous, 2.9 MB/h retail, ≤ 2 s loss on hard kill, rotation, loud failure handling | D23, A10, G15, rung 13 | one `write_all` per 2 s, p95 ≤ 200 µs, inside `instrument_measured` |
| **X13** | Consumption from ECS systems while the frame runs (**C-IV**) | **FOLDED, AND SIMPLIFIED** — `Res<Profiler>` + `ProfiledZone` + a printed latency table. Because the drain and retire run **outside** the schedule, the extension's `ProfilerSet::{Retire,Read}` pair and its ordering edge are **not needed and are dropped** | D25, rungs 11/15 | CPU data is N−1, GPU N−4…N−2, by construction |
| **X14** | "Gameplay code reads its own counters to make decisions" | **SPLIT: half folded, half REFUSED** — windowed statistics driving LOD / dynamic resolution: supported. Same-frame counter readback as a message bus: refused (a shared-line RMW on the hot path or a mid-frame drain; the ECS already has events and resources) | D25, D28 | the game samples its own ECS-owned datum once per frame via `gauge!` |
| **X15** | Debug HUD via `boyko_ui` | **FOLDED** — reference overlay at rung 15, zero-alloc gated with a positive control, `ZoneId` resolved once at setup | D25, G19, rung 15 | none |
| **X16** | Sampling / decimation policy | **DECIDED, ONE FORM REFUSED** — decimation at retention and via the scope bit; **no 1-in-N gate at the call site** (a per-site RMW on a shared line) | D22, D28 | a game wanting 1-in-N implements it in its own code, visibly |
| **X17** | Per-session / per-player identity | **IN** — `session_id`, `run_id`, `build_hash`, opaque `player_tag[16]` the engine never interprets | D23, D26 | 44 B of header |
| **X18** | Save / replay correlation | **IN, at 8 B/record** — `fixed_elapsed_ns` = `FixedTime::elapsed()` (`fixed_time.rs:162`), the kernel's determinism witness | D26, A10 | none |
| **X19** | Multi-process / networked aggregation | **OUT, argued** — needs cross-machine clock correlation D14 refuses to fake on one machine, plus a transport the engine lacks; the merge is a tool over files that already share `session_id` + `fixed_elapsed_ns` | D26 | re-entry condition named |
| **X20** | Live network viewer / remote streaming | **OUT, argued** — the Tracy protocol renamed (D10), plus a socket in the frame loop | D26, D28 | a tailed file answers the same question at zero engine cost |
| **X21** | Remote arm/disarm switch | **ALREADY SERVED** — a network handler calls `world.enable::<ProfilingScope>(e)` like any system | D20, D26 | nothing to build |
| **X22** | Session-scale clock hazard (suspend/resume) | **FOLDED — a hazard a 121-frame horizon cannot meet by itself** — forward-jump detector, window discard, epoch counter, `#[cold]` recalibration, `W9216`, `NotResolved { EpochBreak }` | D3, A2, G21 | one 20 ms hitch after a resume |
| **X23** | Lane capacity in a retail process | **FOLDED** — tier-dependent `const` via `option_env!`, keeping the ring mask an immediate (a runtime capacity would add a hot-path load) | D21, D8 | `boyko_ecs` gains a `build.rs` |
| **X24** | Two panic hooks (profiler + logger) | **REFUSED — one process-global hook** — the logging plan owns it; the profiler exposes `#[cold] flush_on_panic()` for it to call | Invariant 8, D23 | without the logging crate, the ≤ 2 s telemetry loss bound stands unaided |
| **X25** | A second sink thread for telemetry | **REFUSED ON THE NUMBER** — 20-60 µs per 2 s = 0.36 % of one frame in 120; the engine's only threads stay the pool's | D23, open q5 | named escalation trigger: `__telemetry_write` p95 > 200 µs on a real title ⇒ hand off to `boyko_log`'s sink, one thread for both |

**Two parts of the extension are refused as bad ideas for this engine, not deferred:**

1. **The `IsEnabled` observer projection (X9's mechanism).** It cannot be built: the kernel deliberately fires nothing on an enable-bit toggle, and that absence is what buys the O(1) warm toggle (`enable_tag_api.rs:77-88`). Adding a fire would tax every `EnableTag` user in the engine to serve one subsystem. The drain-step projection costs ≤ 320 ns per frame at the 64-scope maximum, is disclosed inside `instrument_measured`, and needs no kernel change.
2. **Same-frame counter readback as an inter-system bus (X14).** It is a shared-line RMW on the emission path or a mid-frame drain, either of which destroys the 12 ns budget the whole design is built around — to duplicate a capability the ECS already has in events and resources.





