# Architecture: Profiling System (`boyko_ecs::profiling` + RHI zone seam) — rev 2

**Status:** design, pre-implementation. **Target file:** `docs/PROFILING-SYSTEM-PLAN.md`.
**Revision:** rev 2, answering 35 review findings (8 BLOCKER, 14 MAJOR, 13 MINOR). Disposition table at the end is the changelog.
**Supersedes:** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` (all three collectors), the `BOYKO_VB_BENCH` / `BOYKO_SV0_BENCH` runner harness bodies, and the private statistics helpers in `crates/boyko_app/src/runner.rs`.

---

## Goal

Replace three hard-coded GPU timestamp enums, two env-var bench harnesses and ~600 lines of hand-rolled statistics with **one ECS-native measurement subsystem** that answers five questions and structurally refuses to answer a sixth.

| Question | Channel | Primary statistic |
|---|---|---|
| What did system `S` cost on the CPU this frame? | `SchedulerCpu` | median / p95 over a 120-frame window |
| Which systems actually ran concurrently? | `SchedulerCpu` + retained intervals | **static compatibility** (from `ConflictGraph`) vs **observed interval overlap** |
| What did GPU pass `P` cost? | `GpuPass` | median / p95 device ticks, per-pair `MEASURED / NOT_BRACKETED / TORN / LOST` |
| How many draws / culled instances / bytes? | `Counter` / `Gauge` | rate-per-frame (counter) or level (gauge) — typed, never interchangeable |
| Where did the frame go? | `Frame` | two lanes (CPU TSC, GPU device ticks), `cpu_gpu_offset = UNCORRELATED` in v1 |
| Is `A` faster than `B` by `Δ`? | contrast API | `Resolved { … }` **or** `NotResolved { … }` — no third return, no bare-delta constructor |

Performance targets (all **measured**, none asserted):

| Metric | Target | Proved by |
|---|---|---|
| zone open+close, channel **on** | ≤ 12 ns median | criterion `zone_cost`, regression gate at +25 % |
| zone open+close, channel **off**, feature on | ≤ 2 ns | same bench, second leg |
| zone, feature **off** | 0 instructions | compile error if the macro names the recorder (G1) |
| drain, armed | ≤ 5 µs/frame at 400 samples | criterion `drain_cost`, and `__drain` is itself a zone |
| drain, disarmed | 1 load + 1 branch | drain body is `if mask == 0 { return }` |
| allocations/frame while armed | 0 | zero-alloc gate test (existing 19-test pattern) |
| profiling `vkCmd*` recorded while disarmed | **0** | command-stream census, two-sided (G5) |
| resident memory, armed | ≤ 7 MiB, allocated once | boot-allocation total `debug_assert` + artifact field |
| GPU readback blocking | **never** | no blocking reader exists in the module (G2a, mechanical) |

---

## Context and constraints

### Subsystems affected

`boyko_ecs` (new `core::profiling`; `SystemMeta` +2 B; `App::update` drain hook; two `Schedule` zone sites) · `boyko_threadpool` (lane-claim API) · `boyko_rhi` (three new device verbs) · `boyko_rhi_vulkan` (`gpu_zone.rs`, then `gpu_timing.rs` retired; `swapchain.rs` present-mode config) · `boyko_render` (dispatcher-pinned GPU retire system) · `boyko_app` (reporter, artifact writer, harness migration).

### Invariants that must survive

1. **Disarmed ⇒ byte-identical recorded command stream.** Enforced by a **command census**, not by image hashes (see D17 / G5).
2. **`SystemMeta` is 256 B** (IM-6 pin, currently a unit test at `system_meta.rs:421`). Field bytes: 232 + 8 (`gpu_intent`) + 1 (`requires_dispatcher`) = 241, next `u16` at offset 242 → 244 ≤ 256, align 32 unchanged. The `zone: ZoneId` field is **unconditional** (no `cfg`), so the pin is configuration-independent. A `const _: () = assert!(size_of::<SystemMeta>() == 256)` is added beside the existing test.
3. **`Schedule::systems` element address stability** — the executor mints raw pointers per dispatch. No field is added to `SystemBox`; no reference is taken across the spawn boundary.
4. **VB-P1d's published numbers keep their meaning** — slots 0/1/2 were defined against `TOP_OF_PIPE` begins; their successor zones declare `GpuStage::TopOfPipe`.
5. **`timestampValidBits` masking before subtraction** stays at the RHI seam (`rhi_impl/device.rs:1249`).
6. **Ticks, not ns, at the seam** (`boyko_rhi/src/device.rs:891-903`): recovering ticks by dividing an `f64` back through `timestampPeriod` launders the measurement through the factor under characterisation.
7. **Principle 0** — the durable store (windows, intervals, frame records, counters, leg summaries) is an ECS `Resource`. The transport rings are a fixed-capacity lock-free structure in the kernel, the same category as `EventBuffer`'s `ThreadLanePair` lanes (`event_buffer.rs:63-140`) and the threadpool deques — Principle 0's own named exception.

### Hard environmental constraints (from the in-tree audit, all re-verified this revision)

- `VK_PRESENT_MODE_FIFO_KHR` is unconditional (`swapchain.rs:199`); `present_mode_supported` already exists (`swapchain.rs:10`, used at `:164`). Any end-to-end wall clock is bounded below by the refresh interval.
- The decidability floor is **not a constant**: 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol. The tree's own definition (`vg_decidability_floor.rs:27-73`) is `FLOOR_SIGMA(3.0) × CV of the WORST statistic of the SHIPPED bench class`, over `DEFAULT_SESSIONS = 7` **separate processes**, repeated `DEFAULT_REPEATS = 3` times (21 processes), with the stated reason: *"a floor established on a different instrument bounds nothing about this one."*
- `Instant::now()` ≈ 25 ns/call, ≈ 60 ns/pair (`profile_spawn.rs:226-240`). Not usable as the zone clock.
- **No engine crate spawns threads.** `thread::spawn` / `thread::Builder` appear only in `boyko_threadpool/{sync,thread_pool}.rs` and two non-render sites. `gpu_timing.rs:442-445` states it: *"Recorder and readback are the same thread today (the runner drives `record_vb` and the post-present readback in one loop)."*
- `WORKER_ID_DISPATCHER` is set only inside `ThreadPool::install` (`tls.rs:24,43`); outside it the host thread is `WORKER_ID_UNATTACHED`, which `current_worker_id_or_dispatcher_lane` maps to lane **0** = worker 0's lane (`tls.rs:69-78`).
- A test/bench binary whose name contains `time` / `update` / `setup` / `install` / `patch` triggers Windows UAC os-error-740.
- `cargo test --workspace --lib` does not build `tests/`; root `cargo check --all-targets` is vacuous (virtual-manifest quirk). Gates use `--workspace` and name the target.
- `debug_assert!` protects nothing in the timing path — it inherits the driver's release profile (`gpu_scene/mod.rs:7498`).
- `ffi.rs:846,849` declares `VK_QUERY_RESULT_64_BIT = 0x1` and `WAIT_BIT = 0x2`. **`WITH_AVAILABILITY_BIT` is `0x0000_0004`** and is not declared. `hostQueryReset` exists as a feature field (`ffi.rs:2716`), is never enabled, and `vkResetQueryPool` is not loaded.
- `goldens/PINS.toml` pins **SHA-256 of a dumped BMP** (`PINS.toml:3`). Image hashes are structurally blind to recorded commands that draw no pixels.

---

## Key decisions

### D1 — Emission: two gates, no allocation, no lock, one 16 B store

`zone!(HANDLE)` expands (feature on) to `let _z = ZoneGuard::open(&HANDLE);` — one `Relaxed` load of a `CachePadded` global mask, one statically-predicted-not-taken branch, one `rdtsc`; on `Drop`, one branch, one `rdtsc`, one 16 B store, one `Release` cursor store. Feature off, the macro expands to nothing **and the recorder function is not in scope**, so a leaked reference is a compile error.

**Why.** NanoLog/Quill measure 7-9 ns with exactly this shape; spdlog measures 242 ns with the same asynchrony but caller-side formatting — the delta is entirely "do no work at the call site". At 400 zones × 60 Hz, 12 ns costs 0.03 % of a frame; 250 ns would cost 6 ms/s. The gate order (`const` ceiling `&&` runtime mask) is `log!`'s verified expansion: short-circuit `&&` over a `const` guarantees the arm and its operands vanish.

**Rejected.** `tracing`/`log` (third-party; `tracing`'s disabled check is a static callsite + two atomic loads, and its layers are `Box<dyn Layer>`). `Instant::now()` (60 ns/pair). `thread_local!` rings (TLS destructors at thread exit — the canonical lock-free-logger bug; the engine already has a pool-owned lane index). An `AtomicBool::swap` once-latch in a reader (`render_path_config.rs:311-313` executes an RMW on a shared line every frame forever once its condition holds).

**Trade-off.** A `mem::forget`ed guard loses its sample silently in release; `ZoneGuard` is `#[must_use]` and a debug-only TLS depth counter (D3a) catches it in debug.

### D2 — Lane taxonomy matches the ACTUAL thread topology; unclaimed threads are refused

The engine has no present thread and no asset thread. The real hazard is that **the host thread is `UNATTACHED` outside `install`** and therefore collapses onto lane 0 — worker 0's lane — precisely while it drives the post-present GPU readback.

```
lane 0..63   workers            (id from CURRENT_WORKER_ID)
lane 64      dispatcher         (WORKER_ID_DISPATCHER, i.e. host thread INSIDE install)
lane 65      host               (claimed by the runner at boot; host thread OUTSIDE install)
lane 66..67  spare claimable
LANE_COUNT = 68
```

Resolution order at emission: worker id `< 64` → that lane; `WORKER_ID_DISPATCHER` → 64; else the TLS-claimed lane if any; **else drop and count** (`unclaimed_drops`, reported unconditionally, even when zero).

One OS thread may hold two lane identities (dispatcher inside `install`, host outside). This is sound: the thread is serial, so each lane still has exactly one writer, and samples carry absolute TSC so the timeline joins without an epoch.

**Rejected.** An MPSC fallback lane (a second ring type, a second drain path, for zero threads). Widening `current_worker_id_or_dispatcher_lane` (it is the event system's contract; changing its sentinel would move event traffic).

### D3 — CPU clock: `rdtsc`, with a CV-reported calibration

`clock::now()` → `_rdtsc()` when `CPUID.80000007H:EDX[8]` (invariant TSC) is set; a QPC-derived tick otherwise, with `boyko-W9207` and a raised quantum. At **arm** (a setup call, `debug_assert!(!is_in_system_run())`), `calibrate()` runs 16 probe pairs over a bounded `CALIB_WINDOW_MS = 20` window, discards probes whose `(rdtsc, Instant)` disagreement exceeds `1.5 × min_disagreement` (Tracy's rejection sampler), and publishes `ticks_per_ns` with **`calib_cv`** (CV over accepted probes) and `calib_rejected`.

**Why CV and not the worst probe:** D11 rejects peak-to-peak because it grows with `n` and cannot reproduce itself; attaching the worst-of-N probe to every printed nanosecond would be the same defect with the opposite verdict. The bounded 20 ms window is stated because arm is a setup hitch, not a frame event.

**Trade-off.** `rdtsc` is not serializing; the OoO engine may move instructions across a bracket. Consequence, printed as a field: the CPU channel's **quantum** is the measured `__cpu_null` median, and no span shorter than it is reported as a number.

#### D3a — `depth` is debug-only and lives in TLS, not in the lane

The rev-1 design had `depth: u16` as a plain field inside a `static` — mutating it through `&'static` without `UnsafeCell` is UB, and no hot-path step incremented it. Removed. Nesting is reconstructed at drain (a lane is single-writer, so its samples form an exact stack). Forgotten-guard detection is `#[cfg(debug_assertions)] OPEN_DEPTH: Cell<u16>` in TLS — zero release cost, no UB, and the drain's `debug_assert!(OPEN_DEPTH == 0)` becomes meaningful. `capacity` is a `const`, not a field.

### D4 — GPU readback is availability-polled and N-frames deferred; `VK_QUERY_RESULT_WAIT_BIT` is removed

```rust
/// NEVER sets VK_QUERY_RESULT_WAIT_BIT. `VK_NOT_READY` maps to Ok(()) with the
/// corresponding availability bits CLEAR — it is a normal outcome, not an error.
fn read_query_pool_pairs_available(
    &self, pool: &A::QueryPool, pair_count: u32,
    scratch: &mut [u64],                 // len >= 4 * pair_count (value + availability per query)
    out_begin_ticks: &mut [u64], out_dur_ticks: &mut [u64],
    out_available: &mut [u8],            // one byte per pair: 1 iff BOTH queries available
) -> Result<(), Self::Error>;
```

`VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x0000_0004` (rev 1 said `0x20`, which is undefined; `0x10` is `WITH_STATUS_BIT_KHR`). The availability output is a **byte slice, not a `u128`** — no fixed width wall.

A frame slot retires when every bracketed pair is available, or when its fence is signalled and `RETIRE_GRACE_FRAMES = 2` further frames have passed. `GPU_RING_DEPTH = 4 > FRAMES_IN_FLIGHT = 2`.

**Why.** Tracy polls with the availability bit and breaks at the first unavailable query; Bevy resolves into a readback buffer and picks it up via `map_async` + `AtomicBool(Release)`. Neither blocks. The two hang classes documented at `gpu_timing.rs:186-203` and `:575-584` exist *only* because the reader blocks — removing the block closes both structurally, and with them the reason the three collectors are separate at all.

**Rejected.** Keeping `WAIT_BIT` + widening the totality epilogue (a device-side patch for a host-side mistake: it records two extra timestamps per unbracketed pass into the stream being measured, and makes termination depend on recorder discipline forever). `VK_QUERY_RESULT_PARTIAL_BIT` (the spec makes an unavailable result *undefined*, not zero).

**Trade-off.** Results arrive 2-4 frames late; a frame is `Pending` until its slot retires. Live display of the current frame's GPU cost is impossible. Accepted — every question here is windowed.

### D5 — The witness survives as a per-pair mark array with a single seal; the epilogue's mechanism becomes the QUANTUM probe

`AtomicU128` does not exist (not stable, not nightly; zero occurrences in the tree), and a hand-rolled 128-bit atomic is `cmpxchg16b`, a full RMW — not the cheap `Release` store the ordering argument assumes. Representation instead:

```rust
struct FrameSlot {
    marks: UnsafeCell<[u8; MAX_GPU_PAIRS]>,  // bit0 = begun, bit1 = ended; single producer
    seal:  AtomicU32,                        // the ONE release edge: stores `frame` after marks
    ...
}
```

The recorder writes marks (plain stores; exactly one thread per slot), then `seal.store(frame, Release)`. Retire does `seal.load(Acquire)`; if it equals the expected frame, the marks are visible. This scales to any pair count with no bitmask width wall (rev 1's `MAX_GPU_ZONES_PER_FRAME = 128` exactly filled a `u128` — the same fixed-width hazard D6 calls out in `VbTimedPass`, moved rather than removed) and costs a plain byte store instead of an atomic OR.

Label is the 2×2 over (witness, availability):

| begun | ended | available at deadline | label |
|---|---|---|---|
| 1 | 1 | yes | `MEASURED` |
| 0 | 0 | – | `NOT_BRACKETED` (this leg does not run that pass) |
| 1 | 0 | – | `TORN` (recorder bug) |
| 1 | 1 | no | `LOST` → **NOT RESOLVED, no number printed** |

**Why the witness is still needed:** availability answers "the GPU wrote this query", not "the recorder bracketed this pass". A pass that never ran and a pass whose queries were never reported are both `available == 0` and mean opposite things. `gpu_timing.rs:432-445`'s argument (a duration cannot distinguish a free pass from a filled one; a begin-offset rule is a heuristic under mixed TOP/BOTTOM stages) is unchanged.

**`LOST` is a state the old design could not express — it hung instead.**

`write_zero_pair`'s mechanism (two back-to-back `BOTTOM` stamps, whose delta *is* the counter lattice) returns as zone `__gpu_null`, emitted once per armed frame. It is **the quantum probe, explicitly not the floor** — see D11.

### D6 — Zone identity: a dense `u16` minted once, single registry, no strings on the emission path

```rust
declare_zone!(VB_EARLY_RASTER,
    name = "vb_early_raster", channel = Channel::GpuPass, kind = ZoneKind::Span,
    stage = GpuStage::BottomOfPipe, group = PartitionGroup::VbRun);
```

expands to `pub static VB_EARLY_RASTER: ZoneHandle { desc: &'static ZoneDesc, id: AtomicU16 }`.

**Minting (dense, no leaked ids).** CAS the site `UNASSIGNED → RESERVED`; the winner stores the desc pointer into `REGISTRY[n]` (`Release`), then `fetch_add`s the global counter and stores the id (`Release`); losers spin in a `#[cold]` loop until the id is non-`RESERVED`. Rev 1's reserve-then-CAS leaked a counter value per lost race, making the id space sparse and firing exhaustion early.

**One registry, one truth.** `static REGISTRY: [AtomicPtr<ZoneDesc>; MAX_ZONES]`. The `Profiler` Resource holds **no desc mirror** (rev 1 had two); the reporter reads `REGISTRY` at report time. Because the desc pointer is stored before the id is published, any reader that sees an id sees its desc.

System zones are pre-registered at `ScheduleBuilder::try_build`, so their emission path never takes the registration branch (the branch is still emitted; it is statically predicted not-taken and never taken).

`MAX_ZONES = 1024` (`big_zone_table` → 4096). Exhaustion is terminal (`boyko-E9201`), mirroring `query_type_registry.rs:124-144` — **but with the warning tier that registry lacks**: `boyko-W9208` fires once at 90 % occupancy.

**Rejected.** defmt-style linker-section interning (the consecutive-address property is an ELF linker-script artifact; this box is windows-gnu/PE-COFF). A fixed `#[repr(u32)]` enum per subsystem (that *is* `VbTimedPass`, whose widening hazard we are removing).

### D7 — The stage table becomes a per-zone declaration, and partition sums are CHECKED

`ZoneDesc.stage: GpuStage` and `ZoneDesc.group: PartitionGroup`. The reporter refuses to sum a group unless **every** member declares `BottomOfPipe` and their intervals are non-overlapping and contained in the group's run bracket; otherwise it prints members individually with `sum = NOT_VALID (mixed stage)`.

**Why.** `begin_stage`'s argument (`gpu_timing.rs:333-365`) is correct and currently enforced by nobody: consecutive `BOTTOM` stamps are prefix-completion times, prefixes nest, so their intervals exactly partition the span; a `TOP` stamp recorded *after* a `BOTTOM` stamp may legally report an earlier time. Today `froxel_total_ns` sums three independent brackets and discloses it only in a prose `NOTE:`. Making it a checked property is the difference between a caveat and an invariant.

**Trade-off.** VB-P1d slots 0/1/2 stay `TopOfPipe` and can therefore never join a partition group. Correct — they never could.

### D8 — Storage: a `Resource`-owned FRAME-MAJOR SoA store; transport rings are a boot slab

**Layout fork, decided with numbers.** Rev 1 indexed `[zone * WINDOW + frame]` and deferred the justification to "measured in practice" — a promise, not a decision.

| Layout | Drain (per frame, hot) | Reporter reduction (cold, once) |
|---|---|---|
| zone-major `[zone*W + f]` | ~400 live zones × 4 columns = **1600 distinct lines ≈ 100 KiB**, over L1d | sequential per zone |
| **frame-major `[f*MAX_ZONES + zone]`** | one row per column = 1024×8 B / 1024×2 B / 1024×4 B ×2 → **≤ 256 lines ≈ 16 KiB**, fits L1d | constant-stride gather (stride 8 KiB), 120 reads per zone per column, hardware stride prefetcher applies |

**Frame-major wins by ~6× on the frequent side**, and the strided side runs `#[cold]` once per report. Decided.

**Transport.** `static LANES: [ZoneLane; 68]` — control blocks in BSS (8.5 KiB); the sample slab is one `Box<[Sample; 68*4096]>` allocated at **first arm** and **never freed** (see D15). Each lane's `buf: AtomicPtr<Sample>` is published `Release` once and never nulled.

**Why the rings are static and not inside the Resource.** The emitters are (a) the executor, running outside any system's param set, (b) worker closures holding only a raw `SystemBox` pointer, and (c) the host thread outside `install`, which has no world. Reaching a Resource from those needs a published `NonNull`, a null check and a world-drop lifetime hazard — to arrive at the same bytes.

**Multi-world.** The rings are process-global; worlds are not. **v1 binds the profiler to exactly one world**: `ProfilerPlugin::build` records the `WorldId` in a global; a second registration is `boyko-E9204`. Enforced at bind time, not assumed.

### D9 — Concurrency = STATIC compatibility vs OBSERVED interval overlap (waves carry no membership)

Rev 1 could not compute its own headline: `Profiler` retained only per-zone per-frame `total/count/min/max`, and per-sample begins were folded away at drain — so an overlap matrix was uncomputable. And `WaveRecord.members: [u64; 4]` silently truncated above 256 systems, understating declared concurrency with no drop counter, in the one channel whose value is that gap.

Both are fixed by moving each half to where the information actually lives:

- **Declared** = the static compatibility matrix, snapshotted from `ConflictGraph` at arm: `compat: Box<[u64]>`, 512×512 bits = 32 KiB. Pair `(i,j)` is compatible iff no access conflict and no ordering edge in either direction. This is the true *declaration*; a wave is only one realisation of it.
- **Observed** = interval overlap over a retained interval ring: `intervals: Box<[Interval; OVERLAP_FRAMES * MAX_SYSTEMS]>`, `OVERLAP_FRAMES = 8`, `MAX_SYSTEMS = 512`, `Interval { begin: u64, dur: u32, _pad: u32 }` = 16 B → **64 KiB**. Written at drain for system-tagged span samples. Eight frames is enough: overlap is a structural property, not a windowed statistic.
- `RoundRecord { frame: u32, round: u16, dispatched: u16, begin: u64, end: u64 }` = 24 B keeps dispatch *shape* only (rounds per frame, wave width, round span). No membership mask, hence **no truncation and no silent wrong answer**. `MAX_ROUNDS_PER_FRAME = 32`; overflow is counted and reported.

`ConcurrencyReport` prints, per compatible pair that both ran: `declared=1 observed_frac=x.xx`, plus the aggregate `serialisation index` = 1 − (Σ observed overlap / Σ declared-compatible-and-both-ran). That number is the scheduler's efficiency; neither half alone says it.

### D10 — Fully in-house. No Tracy stream, no Tracy protocol, v1 or v2

1. `tracy-client` is a C++ client, a build script and a TCP server process — the largest possible dependency against a standing zero-third-party stance.
2. **Tracy's wire format cannot represent the one property this system exists for.** `NOT RESOLVED`, `LOST`, a floor, a measured quantum — none is expressible as a Tracy zone. Exporting would render them as durations, i.e. launder unresolvable deltas back into numbers.
3. Tracy's genuine inventions — availability-polled collection, a rejection-sampled calibration — are techniques, and we take them (D4, D3, D16). The protocol is not the technique.

**Concession.** No free viewer. The artifact is flat TOML with `schema_version`, renderable by a ~60-line script. A v1.2 optional exporter may emit Chrome-trace JSON containing **only `MEASURED` rows** — the dropping is the exporter's purpose, not its limitation.

### D11 — The floor is the tree's floor: cross-process, 3σ, same instrument. The null zones are the QUANTUM, and a quantum is not a floor

Rev 1 substituted an empty bracket, measured within one session, at 1σ — three substitutions, each shrinking the floor, yielding a bound 2-4 orders below the campaign's own measured limit. `resolve` would then have returned `Resolved` on exactly the deltas the audit calls indefensible, with a type signature vouching for it.

**Two distinct quantities, never conflated:**

| Quantity | What it is | How measured | Where it appears |
|---|---|---|---|
| **Quantum** | the instrument's own resolution | `__cpu_null` (empty bracket) median; `__gpu_null` (two back-to-back BOTTOM stamps) median | printed beside every number; a span below its channel's quantum prints `BELOW QUANTUM`, never a value |
| **Floor** | the smallest defensible delta for **this workload, this box, this protocol** | `FLOOR_SIGMA = 3.0 × CV` of the **workload under test**, across `SESSIONS = 7` separate processes, `REPEATS = 3` (the estimator's own stability printed, not averaged) — the `vg_decidability_floor.rs:27-73` protocol verbatim | the only thing `resolve` accepts |

**`Floor` is a type with no cheap constructor:**

```rust
pub struct Floor { rel: f64, provenance: FloorSource, sessions: u32, repeats: u32 }
impl Floor {
    pub fn from_session_file(path: &Path) -> io::Result<Floor>;      // cross-process, 3σ
    pub fn from_aa_control(control: &LegSummary, sigma: f64) -> Floor; // in-sitting A/A of the SAME workload
}
// There is no Floor::from_quantum. A null-zone window cannot become a Floor.
pub fn resolve(a: &LegSummary, b: &LegSummary, floor: Floor) -> Contrast;
```

`resolve` uses `max(floor, quantum_of_channel)`. G3's control (A/A workload) and the floor are now the same object — rev 1 had the gate and the decision using different controls.

**Contrast protocol: ABBA, never ABAB.** With `FRAMES_IN_FLIGHT == 2`, strict alternation aliases the A/B phase perfectly with the frame-in-flight slot — different pool, different UBO ring slot, different staging, forever. ABBA breaks the alias; the cancelled order bias is **reported** (`order_bias_ticks`), not hidden (`sv0_deferred_term_bench.rs:20-72`, generalised).

**No warm-up doctrine.** Warm-up 20 → 100 was tried and reverted as a measured negative (`runner.rs:158-172`): the ramp is ongoing drift, not a settling transient. Instead every window prints `median_first_half` / `median_second_half`, so drift is visible rather than assumed away.

### D12 — Present mode becomes configurable; wall clock is demoted to a labelled, probed observation

`PresentModeConfig { Fifo, Immediate }` (`Mailbox` is declared and returns `Unsupported` until a harness needs it — one code path, not three). Default `Fifo`, so no golden pin moves. Support is **probed** with the existing `present_mode_supported` (`swapchain.rs:10,164`) and the *resolved* mode is recorded in the artifact; an unsupported request falls back to `Fifo` with a loud notice (the `BootError::ValidationUnavailable` precedent: refuse or announce, never silently degrade).

The `Frame` channel's wall clock always carries its bound: `frame_wall_ns=… bound=FIFO(refresh≈16.67ms)` or `bound=none`. **Even under `Immediate`, wall clock stays secondary**: the primary CPU number is the `Schedule::run` span (FIFO bounds the present wait, not the run), and the primary GPU number is the device-tick delta. Removing FIFO buys a non-vacuous *frame* channel, not a better *pass* channel.

**Why at all.** While FIFO is unconditional no wall-clock gate can fail for GPU-side work, and this project treats a gate that cannot fail as a defect — the measured precedent being `-ValidationOn` reporting "clean, 0 messages" for all 22 pins while an illegal `mip_levels: 12` drew zero.

### D13 — Counters and gauges are typed at the WINDOW level, so the wrong statistic is unrepresentable

`ZoneKind ∈ { Span, Counter, Gauge }`, and the accessors are kind-specific:

```rust
fn span(&self, id: ZoneId)    -> Option<SpanWindow<'_>>;
fn counter(&self, id: ZoneId) -> Option<CounterWindow<'_>>;   // None on wrong kind
fn gauge(&self, id: ZoneId)   -> Option<GaugeWindow<'_>>;
```

`rate_per_frame` exists only on `CounterWindow`; `median_ticks` only on `SpanWindow`. Rev 1 put all three on one `ZoneWindow` and **panicked** on the wrong kind — representable, and a runtime panic in a library API against the repo's `Option` / `expect("invariant: ...")` convention. The distinction is now enforced by the type system, which is what D13 claims.

flecs types this (`ecs_gauge_t` vs `ecs_counter_t`, the latter containing its own rate gauge); Unreal types it; Bevy does not, which is exactly why "an average frame count would be nonsensical" is special-cased in a plugin.

**Counter authoring rules — `VbRecordProbe`'s three, promoted to contract** (`passes/vb.rs:86-100`):
1. **Counts originate AT the operation they count** — at the `vkCmd*` call, inside the cull loop — never re-derived on the host. *"A host that re-derives `scopes` from `GBufferScene::vb_occlusion_instances` agrees with itself no matter what this function did — the tautology this campaign has shipped as a gate five times."*
2. **Host memory, not a device buffer.** A device counter adds an allocation, a declared pass, a barrier, a fence wait and a decode to move a number already in a register — and changes the recorded command stream.
3. **What a counter cannot claim is a field of the artifact**, not a prose paragraph.

**Allocation counting.** The 19 zero-alloc gates each install a process-global allocator, which is why they can only be test binaries. **The profiler installs no global allocator.** An opt-in `profiling-alloc` feature in `boyko_app` installs a counting shim feeding the `Counter` channel; off by default, and its perturbation is stated in the artifact when on.

### D14 — Clock correlation is two-tier; tier 1 says `UNCORRELATED` rather than guessing

**Tier 1 (v1, mandatory, zero cost).** GPU spans on a device-tick axis anchored at the frame's first `BOTTOM` stamp; CPU spans on a TSC axis anchored at `Schedule::run` entry. Two lanes, declared unmeasured offset, artifact field `cpu_gpu_offset = UNCORRELATED`.

**Tier 2 (v1.1, gated on `VK_EXT_calibrated_timestamps`).** 32 probes at arm, acceptance threshold `min_deviation × 3/2`, recalibration each drain, `max_deviation_ns` published with every correlated number.

**Why defer.** Every question the audit found being asked is within-domain. The Khronos problem statement is why it cannot be faked: core Vulkan timestamps *"cannot be compared even across separate submits within the same run of an application, as power management events can reset the timer."* An uncalibrated cross-domain offset is not an approximation; it is a fabrication.

### D15 — Lane buffers are allocated once and NEVER freed; disarm is a mask store

Rev 1 freed the lane slab at disarm behind a quiescence argument that covered workers only — the claimed foreign lanes are, by construction, not synchronised with a between-frames dispatcher point, so a foreign emitter that had loaded a non-null `buf` could be mid-store when the `Box` dropped. The stated guard was worse than absent: `is_in_system_run()` (`tls.rs:83`) reads **the calling thread's own TLS** and can never observe another thread. And the cited precedent does not transfer: `ThreadLaneWriter`'s Sync clause 2 (`event_buffer.rs:102-106`) rests on *"`update_events`, which takes `&mut EventDispatcher` — the `&mut` acts as the synchronisation point"*; a `static LANES` has no `&mut` to stand in for that clause.

**Decision: there is no free.** The slab is allocated on the first `arm()` and lives for the process. `arm`/`disarm` only store `CHANNEL_MASK` and reset cursors. `buf` is published once, `Release`, and never nulled. The use-after-free class is deleted by construction rather than argued away.

**Honest consequence:** "disarmed = 9 KiB BSS and nothing else" is true only *before the first arm*. After a first arm, disarmed resident cost is 4.25 MiB of retained slab plus 2.4 MiB of store. Stated in the artifact and in the target table.

### D16 — The instrument is outside its own primary number, and its cost is reported

Rev 1 put the drain at the top of `Schedule::run` and named the `Schedule::run` span the primary CPU number — the system perturbing exactly the figure it advertises, with no per-frame instrumentation total reported.

- The drain runs in `App::update`, **before** `Schedule::run` opens its span. It is outside the primary number entirely.
- The drain and the reporter are themselves zones (`__drain`, `__report`) on the dispatcher lane.
- Each `FrameRecord` carries `run_gross`, `instrument` (Σ `__drain` + `__report` + `__cpu_null` + estimated per-zone emission cost = `zone_count × measured_zone_cost`) and `run_net = run_gross − instrument_inside_run`. All three are printed.

### D17 — Disarmed byte-identity is proved by a COMMAND CENSUS, not by image hashes

`goldens/PINS.toml` pins SHA-256 of a dumped BMP (`PINS.toml:3`). A `vkCmdResetQueryPool` plus two `vkCmdWriteTimestamp`s change zero pixels, so rev 1's G5 ("record one command on the disarmed path → pins move") was false as written and would have passed in exactly the state it forbids.

**Mechanism:** `RecordCensus { profiling_cmds: u32, query_resets: u32, timestamps: u32 }`, a `&mut` host parameter threaded into the recorders, incremented **at the `vkCmd*` call sites** (D13 rule 1), exactly like `VbRecordProbe` (`passes/vb.rs:107-156`). Two-sided gate:

- disarmed frame ⇒ `profiling_cmds == 0` (and every sub-counter 0);
- armed frame ⇒ `timestamps >= 2` (the `__gpu_null` pair alone guarantees it).

A one-sided assertion is passable by a recorder that records nothing at all; the second clause is what makes the first non-vacuous. Golden pins remain as a *secondary* check on pixels, with the explicit note that they are structurally incapable of the command claim.

---

## Data structures

```rust
// ─────────────────────── boyko_ecs::core::profiling ───────────────────────

#[repr(u8)] pub enum Channel { SchedulerCpu=0, GpuPass=1, Counter=2, Frame=3,
                               User0=4, User1=5, User2=6, User3=7 }
#[repr(u8)] pub enum ZoneKind { Span, Counter, Gauge }
#[repr(u8)] pub enum GpuStage { TopOfPipe, BottomOfPipe, NotGpu }
#[repr(u8)] pub enum Unit     { Ticks, Count, Bytes, Ratio }

/// Immutable, `&'static`, one per site. NEVER on the emission path.
#[repr(C)]
pub struct ZoneDesc {
    pub name: &'static str,   // REQUIRED by declare_zone! -> cannot be forgotten (the property
                              // VbTimedPass::label() bought with a hand-maintained table)
    pub file: &'static str, pub line: u32,
    pub channel: Channel, pub kind: ZoneKind, pub stage: GpuStage, pub unit: Unit,
    pub group: u16,           // PartitionGroup; 0 = none
    pub system_tag: bool,     // true => intervals retained for overlap analysis (D9)
}

#[repr(C)] pub struct ZoneHandle { desc: &'static ZoneDesc, id: AtomicU16 }

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

/// Writer and reader halves on SEPARATE lines — the ThreadLanePair shape the kernel
/// already pins at 128 B (event_buffer.rs:51-59). All mutable state is atomic:
/// no UnsafeCell, no plain field mutated through `&'static` (rev 1 had both).
#[repr(C, align(64))]
struct ZoneLaneWriter {
    buf:      AtomicPtr<Sample>,  // published ONCE at first arm (Release), never nulled (D15)
    write:    AtomicU32,          // Relaxed read by the sole owner; Release store after the bytes
    overflow: AtomicU32,          // dropped samples. NEVER silent.
    _pad:     [u8; 48],
}
#[repr(C, align(64))]
struct ZoneLaneReader { read: AtomicU32, _pad: [u8; 60] }
#[repr(C, align(64))]
struct ZoneLane { w: ZoneLaneWriter, r: ZoneLaneReader }
const _: () = assert!(size_of::<ZoneLane>() == 128);

pub const LANE_WORKERS: usize = 64;      // mirrors boyko_threadpool::MAX_WORKERS
pub const LANE_DISPATCHER: usize = 64;
pub const LANE_HOST: usize = 65;         // host thread OUTSIDE install (D2)
pub const LANE_SPARE_0: usize = 66;      // 2 spare claimable
pub const LANE_COUNT: usize = 68;
pub const LANE_CAPACITY: u32 = 4096;     // 64 KiB/lane; 4.25 MiB total. A const, not a field.

static CHANNEL_MASK: CachePadded<AtomicU32>;         // 0 == disarmed. Own line, read-mostly.
static LANES: [ZoneLane; LANE_COUNT];                // 8.5 KiB BSS
static REGISTRY: [AtomicPtr<ZoneDesc>; MAX_ZONES];   // THE single registry (D6)

/// Dispatch SHAPE only — no membership mask, hence no truncation (D9).
#[repr(C)]
pub struct RoundRecord { frame: u32, round: u16, dispatched: u16, begin: u64, end: u64 }

/// Retained per-system interval for observed-overlap analysis (D9).
#[repr(C)] pub struct Interval { begin: u64, dur: u32, _pad: u32 }

#[repr(u8)] pub enum FrameState { Pending, Sealed, Partial }

#[repr(C)]
pub struct FrameRecord {
    frame: u32, state: FrameState, flags: u8, rounds: u16,
    cpu_begin: u64, cpu_end: u64,      // Schedule::run span, TSC
    run_gross: u64, instrument: u64,   // D16 — the instrument's own cost, always printed
    gpu_total: u64,                    // sum of MEASURED partition-valid GPU zones
    wall_ns: u64,                      // labelled with its present-mode bound (D12)
    drops: u32, _pad: u32,
}                                       // 72 B

// ───────────────────── the ECS Resource (Principle 0) ─────────────────────

pub const WINDOW: usize = 120;                  // 2 s at 60 Hz
pub const MAX_ZONES: usize = 1024;              // `big_zone_table` -> 4096
pub const MAX_SYSTEMS: usize = 512;
pub const OVERLAP_FRAMES: usize = 8;
pub const MAX_ROUNDS_PER_FRAME: usize = 32;
pub const MAX_LEGS: usize = 8;
pub const CONTRAST_ZONES: usize = 16;

/// FRAME-MAJOR columns: index [frame * MAX_ZONES + zone]  (D8, decided with numbers).
pub struct Profiler {
    total: Box<[u64]>,   // 1024*120*8 = 960 KiB. Span: Σ ticks. Counter/Gauge: last level.
    count: Box<[u16]>,   // 240 KiB
    min:   Box<[u32]>,   // 480 KiB
    max:   Box<[u32]>,   // 480 KiB
    label: Box<[u8]>,    // 120 KiB — MEASURED/NOT_BRACKETED/TORN/LOST
                         //                                        columns total ≈ 2.23 MiB

    frames:    Box<[FrameRecord]>,  // [WINDOW]                        8.4 KiB
    rounds:    Box<[RoundRecord]>,  // [WINDOW * MAX_ROUNDS_PER_FRAME] 90 KiB
    intervals: Box<[Interval]>,     // [OVERLAP_FRAMES * MAX_SYSTEMS]  64 KiB
    compat:    Box<[u64]>,          // MAX_SYSTEMS^2 bits              32 KiB
    legs:      Box<[LegSummary]>,   // [MAX_LEGS * CONTRAST_ZONES]      4 KiB

    frame_begin_tsc: Box<[u64]>,    // [WINDOW] — the attribution cut (A2, M1)
    clock:  ClockCalibration,       // ticks_per_ns, calib_cv, calib_rejected
    quantum:[u64; 8],               // per channel, from __cpu_null / __gpu_null (D11)
    cursor: u32,
    drops:  DropCounters,           // lane_overflow, unclaimed, late, rounds, gpu_budget
}
```

**Sizing, armed:** lanes 4.25 MiB · columns 2.23 MiB · frames/rounds/intervals/compat/legs 0.19 MiB · GPU host state ~4 KiB · device pools 4 × 256 × 8 B = 8 KiB. **≈ 6.7 MiB, allocated once at first arm, never freed** (D15). Before the first arm: 8.5 KiB BSS + the registry array.

```rust
// ──────────────── boyko_rhi_vulkan::present::gpu_zone ────────────────

pub struct GpuZoneRecorder {
    pools: [VulkanQueryPool; GPU_RING_DEPTH],
    slots: [FrameSlot; GPU_RING_DEPTH],
    next:  u32,
}
#[repr(C)]
struct FrameSlot {
    marks:   UnsafeCell<[u8; MAX_GPU_PAIRS]>,  // bit0 begun, bit1 ended — single producer (D5)
    zone_of: Box<[u16; MAX_GPU_PAIRS]>,        // pair -> ZoneId, boot-allocated
    seal:    AtomicU32,                        // THE release edge; == frame when marks are valid
    frame:   u32,
    used_pairs: u16,                           // bump allocator
    fence_seen: AtomicBool,
    grace: u8,
    needs_cmd_reset: bool,                     // set when host reset is unavailable (D18)
}
pub const MAX_GPU_PAIRS: usize = 128;   // 256 queries — Bevy's QuerySet size
const _: () = assert!(MAX_GPU_PAIRS * 2 <= QUERY_POOL_WIDTH);
pub const GPU_RING_DEPTH: usize = 4;
pub const RETIRE_GRACE_FRAMES: u8 = 2;
```

### D18 — `hostQueryReset` is an optimisation with a fully specified fallback

Enabling `VkPhysicalDeviceHostQueryResetFeatures` at device creation records **no commands** and changes no frame; it is a `pNext` bit, and the goldens are unaffected. It is enabled when the physical device advertises it.

Nothing establishes that this box's driver does (`ffi.rs:2716` shows the field exists and is never enabled). **The design does not depend on it.** Fallback, specified rather than named: a slot that retires without host reset sets `needs_cmd_reset`; slot recycling refuses that slot until an armed frame issues `vkCmdResetQueryPool` for it at the frame top — the exact site the current code already uses, outside any render scope, satisfying `VUID-vkCmdResetQueryPool-renderpass`. With `GPU_RING_DEPTH = 4` and `FRAMES_IN_FLIGHT = 2` there is always a clean slot, so the fallback never stalls. Host reset merely removes the one-frame recycle latency.

---

## Public API

```rust
// ── emission (feature-off expands to nothing; the recorder is not in scope) ──
declare_zone!(IDENT, name = "...", channel = ..., kind = ..., stage = ..., group = ...);
zone!(IDENT);                       // RAII
#[must_use] zone_open!(IDENT) -> ZoneGuard;   // cross-function brackets
counter!(IDENT, value: u64);
gauge!(IDENT, value: u64);

// ── lanes (boyko_threadpool) ──
pub struct ProfilerLane(u16);
pub fn claim_profiler_lane() -> Option<ProfilerLane>;   // None when all spares are taken

// ── session control ──
pub struct ProfilerConfig { pub channels: u32, pub window: u32 }
pub fn arm(world: &mut EcsMaster, cfg: ProfilerConfig) -> Result<(), ProfilerError>;
pub fn disarm(world: &mut EcsMaster);   // a mask store; frees nothing (D15)

// ── reading — kind-specific, so the wrong statistic is unreachable (D13) ──
impl Profiler {
    pub fn span(&self, id: ZoneId)    -> Option<SpanWindow<'_>>;
    pub fn counter(&self, id: ZoneId) -> Option<CounterWindow<'_>>;
    pub fn gauge(&self, id: ZoneId)   -> Option<GaugeWindow<'_>>;
    pub fn by_name(&self, name: &str) -> Option<ZoneId>;          // #[cold], reporter only
    pub fn frame(&self, back: u32)    -> Option<&FrameRecord>;    // 0 = newest SEALED
    pub fn rounds(&self, back: u32)   -> &[RoundRecord];
    pub fn concurrency(&self)         -> ConcurrencyReport<'_>;   // declared vs observed (D9)
    pub fn quantum(&self, ch: Channel)-> u64;
    pub fn drops(&self)               -> DropCounters;
    pub fn clock(&self)               -> ClockCalibration;
}

pub struct SpanWindow<'a> { /* borrows the frame-major columns */ }
impl<'a> SpanWindow<'a> {
    pub fn median_frame_ticks(&self) -> Option<u64>;   // over per-frame TOTALS
    pub fn p95_frame_ticks(&self)    -> Option<(u64, u64, u64)>; // (p95, lo, hi) order-stat span
    pub fn mean_frame_ticks(&self)   -> Option<f64>;   // O(1), cached sum
    pub fn per_sample_min_max(&self) -> Option<(u32, u32)>;      // distinct unit, distinct name
    pub fn halves(&self) -> (Option<u64>, Option<u64>);          // drift, always printed
    pub fn labels(&self) -> LabelCensus;
    pub fn n(&self) -> u32;
}
impl<'a> CounterWindow<'a> { pub fn rate_per_frame(&self) -> Option<f64>; pub fn level(&self) -> u64; }
impl<'a> GaugeWindow<'a>   { pub fn median(&self) -> Option<u64>; pub fn min_max(&self) -> Option<(u64,u64)>; }

// ── contrast: the ONLY way a delta leaves this system ──
pub struct Floor { /* rel, provenance, sessions, repeats */ }
impl Floor {
    pub fn from_session_file(path: &Path) -> io::Result<Floor>;
    pub fn from_aa_control(control: &LegSummary, sigma: f64) -> Floor;
}                                       // no Floor::from_quantum — a quantum is not a floor

pub enum Contrast {
    Resolved    { median_delta_ticks: i64, p10: i64, p90: i64, n: u32,
                  floor_ticks: u64, quantum_ticks: u64, order_bias_ticks: i64, control_cv: f32 },
    NotResolved { median_delta_ticks: i64, p10: i64, p90: i64, n: u32,
                  floor_ticks: u64, quantum_ticks: u64, order_bias_ticks: i64, control_cv: f32 },
}
pub fn resolve(a: &LegSummary, b: &LegSummary, floor: Floor) -> Contrast;

pub struct ContrastPlan { /* ABBA sequence + leg boundaries */ }
impl ContrastPlan {
    pub fn abba(rounds: u32, frames_per_leg: u32, zones: &[ZoneId]) -> Self;
    pub fn next_leg(&mut self) -> Option<Leg>;      // the CALLER applies the A/B configuration
    pub fn seal_leg(&mut self, p: &mut Profiler);   // folds the live window into a LegSummary
    pub fn summaries(&self) -> &[LegSummary];
}

// ── artifact (incremental: one [[window]] table appended per report) ──
pub fn append_artifact(p: &Profiler, path: &Path) -> io::Result<()>;   // #[cold]

// ── diagnostics seam: the single site the logging plan re-points (m13) ──
pub(crate) fn emit_diag(code: DiagCode, fields: &[(&'static str, DiagValue)]);  // #[cold]

// ── RHI seam ──
fn read_query_pool_pairs_available(&self, pool: &A::QueryPool, pair_count: u32,
    scratch: &mut [u64], out_begin_ticks: &mut [u64], out_dur_ticks: &mut [u64],
    out_available: &mut [u8]) -> Result<(), Self::Error>;
fn reset_query_pool_host(&self, pool: &A::QueryPool, first: u32, count: u32)
    -> Result<(), Self::Error>;
fn host_query_reset_supported(&self) -> bool;
```

**Deliberately absent:** any function returning a bare delta; any ns value without its `calib_cv`; any GPU reader that can block; any accessor that panics on the wrong `ZoneKind`.

---

## Algorithms for critical paths

### A1 — `ZoneGuard::open` / `Drop`

```
open:  1. CHANNEL_MASK.load(Relaxed)  -> test bit; not taken -> NULL guard, return
       2. HANDLE.id.load(Relaxed); UNASSIGNED/RESERVED -> #[cold] register (D6)
       3. rdtsc
       4. guard = { begin, id, lane }        // lane from a TLS Cell<u16>, one load
drop:  5. id == DISABLED -> return
       6. rdtsc; d = now - begin;  if d > u32::MAX -> #[cold] emit Extension sample, set flag
       7. buf = w.buf.load(Acquire); idx = w.write.load(Relaxed)
       8. idx - r.read.load(Acquire) >= LANE_CAPACITY -> #[cold] overflow.fetch_add(1); return
       9. store 16 B at buf[idx & MASK]
      10. w.write.store(idx + 1, Release)
```

Complexity O(1); ~6 instructions + 2 `rdtsc` armed, 1 load + 1 predicted branch disarmed. Cache: a monotone cursor, 4 samples per line, ~0.25 misses/sample, write-allocated. **No non-temporal store** — the drain reads these bytes within one frame, so evicting them is strictly worse. **No software prefetch** — the hardware stride prefetcher already covers a monotone cursor, and an extra instruction on the hottest path buys nothing. Branches: two, both `#[cold]`-biased. SIMD: none wanted (a 16 B store is one instruction).

### A2 — Drain (`App::update`, before `Schedule::run`, for CLOSED frames only)

Frame attribution (rev 1 left it undefined; `f = cursor` silently mixed frames, legs and late arrivals):

```
cut = frame_begin_tsc[current]              // samples at or after `cut` belong to the live frame
for lane in 0..LANE_COUNT:
    w = LANES[lane].w.write.load(Acquire)   // publishes every sample byte below w
    r = LANES[lane].r.read.load(Relaxed)    // the dispatcher is the sole consumer
    for i in r..w:
        s = buf[i & MASK]
        if s.begin >= cut { stop this lane }         // leave the live frame's samples in the ring
        f = merge_walk(frame_begin_tsc, s.begin)     // O(1) amortised: a lane is TSC-monotone
        if f is older than the retained window { drops.late += 1; continue }
        match kind:
          Span    -> total[f*MAX_ZONES+z] += d; count += 1; min/max; if desc.system_tag
                     -> intervals[(f % OVERLAP_FRAMES)*MAX_SYSTEMS + sys] = {s.begin, d}
          Counter -> total[..] = s.begin (level)
          Gauge   -> gauge fold
    LANES[lane].r.read.store(w, Release)
```

- Complexity O(samples); ~400/frame → 6.4 KiB read, ≤ 256 lines written (frame-major, D8). Under 5 µs, and `__drain` measures itself.
- Cache: lane reads strictly sequential; column writes scattered **inside one 4 KiB row per column** — the whole point of frame-major.
- Branching: one 3-way jump table on `kind`, plus the cut test.
- **Sealing** (rev 1 never stated it): a frame becomes `Sealed` when its drain completes **and** (`GpuPass` disarmed **or** its GPU slot retired). If neither holds after `GPU_RING_DEPTH + RETIRE_GRACE_FRAMES` frames it becomes `Partial`. So `frame(0)` is never permanently `None` with the GPU channel off.

### A3 — GPU slot retire (dispatcher-pinned system, `requires_dispatcher`)

```
for slot in ring where slot.in_flight:
    read_query_pool_pairs_available(...)          // never blocks; VK_NOT_READY -> Ok, bits clear
    if avail covers every bracketed pair: publish MEASURED; retire
    else if slot.fence_seen && slot.grace == 0:
        marks = (slot.seal.load(Acquire) == slot.frame) ? read marks : all-zero
        per pair: (1,1,1)=>MEASURED (0,0,_)=>NOT_BRACKETED (1,0,_)=>TORN (1,1,0)=>LOST
        retire PARTIAL; emit_diag(W9205) per LOST
    else: slot.grace -= 1
    on retire: reset_query_pool_host(..) if supported else set needs_cmd_reset (D18)
```

O(GPU_RING_DEPTH × pairs) = 512/frame. **Termination proof:** every slot retires either on availability or on `fence_seen && grace == 0`; `fence_seen` comes from the existing `submission_epoch` fence gate (`frame_driver.rs:265`), which is signalled or the frame never completes at all. No path waits on a query.

**Thread pinning** (rev 1 specified it twice, incompatibly): retire is a `requires_dispatcher` system, so `ResMut<Profiler>` is touched only on the dispatcher, matching the ownership table. The recorder is the same thread today (`gpu_timing.rs:442-445`); the `seal` Release/Acquire pair states the handoff so a future threaded recorder is a documented edge rather than a silent race.

### A4 — Window reduction, median, overlap (reporter, `#[cold]`)

- Reduction: strided gather, stride `MAX_ZONES * 8 B`, 120 reads per zone per column; AVX2 8-wide over the gathered scratch.
- Median/p95: copy 120 values into stack scratch, sort, index. p95 at n=120 is the 114th order statistic — printed with its neighbours (`p95_lo`, `p95_hi`) so its rank uncertainty is on the page rather than implied.
- Overlap: per compatible pair that both ran, interval intersection over the `intervals` ring — SoA `u64` compare, 4-wide. O(pairs that actually ran × OVERLAP_FRAMES), not O(S²) per frame.

### A5 — Leg sealing (contrast)

`ContrastPlan::seal_leg` folds the current window's ≤ 16 subscribed zones into a `LegSummary { zone, median, p95, n, labels, first_half, second_half }` (32 B) in the `legs` arena. `resolve` consumes **summaries**, never live windows — rev 1's `ZoneWindow` borrowed the live ring, so leg A's data was overwritten before leg B ended. The A/B *configuration change* is applied by the caller (it must be — it is configuration); the plan owns only the sequence and the boundary signal.

### A6 — Floor session (offline, N processes)

The `Floor` protocol is `vg_decidability_floor.rs`'s, generalised: run the **same** workload class in `SESSIONS = 7` separate processes, `REPEATS = 3` times, take `3.0 × CV` of the worst subscribed statistic, print all three repetition floors (never their average), write `docs/PROFILING-FLOOR.md`. `Floor::from_session_file` reads it.

---

## Multithreading model

| Datum | Sharing | Writer | Reader |
|---|---|---|---|
| `CHANNEL_MASK` | shared, read-mostly, `CachePadded` | dispatcher at arm/disarm | every emitter, `Relaxed` |
| `LANES[n].w.buf` | shared pointer | dispatcher at **first arm only** (`Release`) | lane owner (`Acquire`) |
| `LANES[n]` sample bytes | **single writer** = lane owner | lane owner | dispatcher at drain |
| `LANES[n].w.write` | 1W/1R | lane owner (`Release`) | dispatcher (`Acquire`) |
| `LANES[n].r.read` | 1W/1R | dispatcher (`Release`) | lane owner (`Acquire`) |
| `REGISTRY[i]` | shared | first executor (`Release`) | reporter (`Acquire`) |
| `ZoneHandle.id` | shared | CAS `UNASSIGNED→RESERVED→id` | everyone, `Relaxed` |
| `Profiler` | dispatcher-only | drain / retire / reporter | `Res<Profiler>` systems |
| `FrameSlot.marks` | 1W/1R | recorder (plain stores) | retire, gated by `seal` |
| `FrameSlot.seal` | 1W/1R | recorder (`Release`) | retire (`Acquire`) |

**Ordering, each justified.** `CHANNEL_MASK` `Relaxed` — it gates only itself; a worker seeing a stale value records or skips one frame's samples, which is not a correctness property, and `Acquire` would forbid nothing while costing a fence off x86. `w.buf` `Release`/`Acquire` — the slab's initialisation must happen-before any write through the pointer; this is the only pointer-carrying edge. `w.write` `Release`/`Acquire` — the sole publication edge for sample bytes, the same edge `EventBuffer::write_len` uses (`event_buffer.rs:79-81`). `r.read` `Release`/`Acquire` — publishes "these slots are reusable" before the producer may overwrite. `ZoneHandle.id` `AcqRel`/`Acquire` — the desc store must be visible before the id. `FrameSlot.seal` `Release`/`Acquire` — one edge for the whole mark array, which is why a 128-bit atomic is not needed. **No `SeqCst` anywhere.**

**Data-race freedom.**
1. *Sample bytes.* Exactly one writer per lane by construction (D2): workers write `LANES[worker_id]`, the dispatcher `LANES[64]`, the host thread its claimed lane, unclaimed threads nothing. One OS thread holding two lane identities is serial, so each lane still has one writer. Producer touches `[write, read + CAPACITY)`; consumer touches `[read, write)`; disjoint given step 8. Textbook Lamport SPSC — no CAS, no ABA (monotone `u32` cursors, masked indexing).
2. *Cursor wrap.* `u32` wraps after 4.3 G samples ≈ 49 days armed at 400/frame/60 Hz; the masked-index + unsigned-difference form is correct across one wrap.
3. *False sharing.* `ZoneLane` = 128 B with halves on distinct lines (`const _` pinned); `CHANNEL_MASK` `CachePadded`; lanes 128 B apart.
4. *Store.* `Profiler` is dispatcher-only for mutation; `Res<Profiler>` readers are serialised against `ResMut<Profiler>` by the existing conflict graph. **No new synchronisation is introduced.**
5. *Teardown.* There is none (D15): the slab is never freed, `buf` is never nulled. The rev-1 use-after-free window does not exist, and no cross-thread quiescence claim is needed. `is_in_system_run()` is used **only** as a same-thread setup assertion (its actual semantics — `tls.rs:83` reads the calling thread's TLS), never as a cross-thread barrier.
6. *`Send`/`Sync`.* `ZoneLane`: manual `unsafe impl Sync` with three SAFETY clauses mirroring `ThreadLaneWriter`'s (`event_buffer.rs:93-110`), adapted: (a) single writer per lane, enforced by D2's resolution order; (b) the consumer is the dispatcher and touches only `[read, write)`; (c) the atomics cover the cursors, and the sample bytes are covered by the `write` Release/Acquire edge — **not** by a `&mut` synchronisation point, because a `static` has none. `FrameSlot`: `Sync` justified by the single-producer + `seal` edge. `ZoneGuard` is `!Send` via `PhantomData<*const ()>` — it carries a lane index bound to the current thread, so it cannot cross a `spawn`.
7. *Panic.* `ZoneGuard::drop` runs during unwind, so a panicking system's zone closes. Moot in practice: `boyko_threadpool/worker.rs:157-168` aborts on worker panic. Stated so nobody relies on it.

**Partitioning.** CPU zones partition by lane (no stealing, no redistribution, no contention). GPU zones partition by frame slot; exactly one thread touches a slot at a time. Reporter is single-threaded and `#[cold]`.

---

## Integration

| File | Change |
|---|---|
| `boyko_ecs/.../system/system_meta.rs` | `+ pub(crate) zone: ZoneId` (u16, unconditional, offset 242); `+ const _: () = assert!(size_of::<SystemMeta>() == 256)` |
| `boyko_ecs/.../schedule/schedule.rs` | `zone!` around `run_unsafe` (`:1267`) and the dispatcher-inline path (`:1108`); `RoundRecord` emission after the `to_spawn` loop (`:1222`) |
| `boyko_ecs/.../schedule/schedule_builder.rs` | mint each system's `ZoneId` at `try_build`; snapshot the `ConflictGraph` compatibility matrix at arm |
| `boyko_ecs/.../app/app.rs` | drain call in `update`, **before** `Schedule::run` (D16) |
| `boyko_threadpool/src/tls.rs` | `+ claim_profiler_lane()`, `+ PROFILER_LANE: Cell<u16>`, `+ #[cfg(debug_assertions)] OPEN_DEPTH` |
| `boyko_rhi/src/device.rs` | `+` three verbs; mark `read_query_pool_ns`/`_ticks`/`_pairs_ns` **FROZEN — no new callers** |
| `boyko_rhi_vulkan/src/ffi.rs` | `+ VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x0000_0004`; `+ PfnVkResetQueryPool` |
| `boyko_rhi_vulkan/src/device.rs` | enable `hostQueryReset` when advertised; load `vkResetQueryPool`; expose the capability |
| `boyko_rhi_vulkan/src/present/gpu_zone.rs` | **new** — `GpuZoneRecorder`, slot ring, retire, 2×2 label |
| `boyko_rhi_vulkan/src/present/passes/vb.rs` | `TsWitness` → `GpuZoneWitness` (mark array + seal); `write_zero_pair` + epilogue gap-fill deleted at the retirement rung; `RecordCensus` counters at the `vkCmd*` sites |
| `boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | 4 + 1 brackets ported |
| `boyko_rhi_vulkan/src/present/swapchain.rs` | `PresentModeConfig`; `:199` becomes a probed choice with a loud fallback |
| `boyko_rhi_vulkan/src/present/gpu_timing.rs` | **retired at rung 7**, not before |
| `boyko_render/src/profiling_bridge.rs` | **new** — the dispatcher-pinned retire system |
| `boyko_app/src/runner.rs` | harness bodies + statistics helpers deleted **at rung 7**, with their consumers migrated in the same commit |
| `boyko_app/src/gpu_scene/mod.rs` | env arming → `ProfilerConfig`; the single `vb_timing_for_frame`-shaped predicate becomes a channel test |
| `boyko_app/src/profiling/{report,artifact}.rs` | **new** |

**`Arena` / `ComponentPool` / `UnitId`: untouched, deliberately.** The profiler stores no per-entity data, so two-level addressing is not involved; routing 4.25 MiB of transport through `ComponentPool` would buy nothing and put a growth path on the emission side.

**Diagnostic codes (block `92xx`).** `E9201` registry exhausted · `W9202` GPU pair budget exhausted · `W9203` lane overflow / unclaimed drops · `E9204` profiler already bound to another world · `W9205` zone LOST · `W9206` contrast NOT RESOLVED · `W9207` invariant TSC absent · `W9208` registry ≥ 90 % · `W9209` late samples dropped. All are emitted through the single `emit_diag` seam (today `eprintln!`), which is the one site the logging plan re-points; the profiler never prints from an emission path — lane overflow is an `AtomicU32` counted at the site and *reported* at drain.

---

## Implementation plan — every rung compiles the workspace alone

Rev 1's ladder deleted `VbTimestampCollector` / `VbTimedPass` / `write_zero_pair` at PR-5 while ~31 direct references over 6 files (and ~232 references over 18 files across the whole bench surface) still named them, and never listed a single test file as moving. The ladder is now **additive first, subtractive once**.

| Rung | Content | Compiles alone because |
|---|---|---|
| **1** | `profiling/{channel,zone,sample,lane,clock,macros}.rs`; `CHANNEL_MASK`, `LANES`, `REGISTRY`; `claim_profiler_lane`. No integration. | purely additive |
| **2** | `store.rs` (frame-major columns), `drain.rs`, `arm`/`disarm`, `ProfilerPlugin`, world-bind check | additive |
| **3** | `SystemMeta.zone` + const-assert; id minting at `try_build`; two `zone!` sites; `RoundRecord`; `compat` snapshot; `intervals`; `ConcurrencyReport` | one field in tail padding, two zone sites |
| **4** | RHI seam: three verbs + Vulkan impls + `ffi.rs` constants + Mock defaults and their pinning tests. **No consumer.** | old readers untouched |
| **5** | `gpu_zone.rs` + `RecordCensus`. VB brackets **dual-recorded**: both `VbTimestampCollector` and `GpuZoneRecorder` armed, independently. Cross-check test asserts the two agree within one quantum. | both collectors exist; every existing test still compiles and passes |
| **6** | gbuffer + SV0 dual-recording; the R0 harness reads the new channel while the old one still exists | additive |
| **7 (the single subtractive rung)** | Delete `gpu_timing.rs`, the runner harness bodies and statistics helpers **and** migrate every consumer in the same commit: `vb_bench_totality_gate.rs` → **retired** (its mechanism is gone; replaced by G2a/G2b) · `vb_bench_query_validation.rs` → migrated to the census gate · `vb_p1d_cull_shade_bench.rs`, `sv0_deferred_term_bench.rs`, `sv0_adequacy.rs`, `sv0_oracle/mod.rs`, `vb_occ_dense/mod.rs`, `vb_mesh.rs`, `vg_occ_split_timing.rs`, `vg_decidability_floor.rs` → read the artifact instead of parsing stdout · `window_present_gbuffer.rs`, `software_ray_baseline_cost.rs`, `ddgi_probe_gi_cost.rs` → migrated to zones · `occlusion_force.rs`, `handle.rs` → updated | one commit, workspace green before and after |
| **8** | floor session, contrast, reporter, artifact, present mode, counters at `vkCmd*` sites, optional `profiling-alloc` | additive |
| **9 (v1.1)** | `VK_EXT_calibrated_timestamps` + rejection sampler; `cpu_gpu_offset` becomes a number with `max_deviation_ns` | additive |

Ordering constraints: 4 before 5 (seam before consumer); 5 and 6 before 7 (dual-record before retire — the cross-check is what licenses the deletion); 8 after 7.

---

## Metrics and validation

### Gates that can be shown RED

| # | Gate | RED variant |
|---|---|---|
| **G1** | Feature off ⇒ zero cost: the recorder is `#[cfg(feature="profiling")]`, so a macro expansion naming it under feature-off is a **compile error** | remove the `cfg` from the else-arm → workspace fails to build |
| **G2a** | **No blocking reader exists**: a source gate asserts `WAIT_BIT` appears nowhere in `gpu_zone.rs` / `profiling/**` | add `WAIT_BIT` to `gpu_zone.rs` → gate fails **without hanging**. Hang-freedom is proved structurally, not by a hanging test — this repo has no kill-after-timeout pattern (`vb_bench_totality_gate.rs:44-53`), so a red that manifests as a hung CI job is not a showable red |
| **G2b** | **Label positive control**: a deliberately unbracketed pass yields `NOT_BRACKETED` **and** a bracketed pass in the same frame yields `MEASURED` with a non-zero duration | a stub labelling everything `NOT_BRACKETED` fails the second clause. Rev 1's G2 passed such a stub |
| **G2c** | **Availability truth control**: poll a pool immediately after recording and before fence-wait ⇒ `available == 0` for every pair; poll after the fence ⇒ `available == 1` for bracketed pairs and `0` for a pair never written | flip `WITH_AVAILABILITY_BIT` to a wrong/undefined value → availability words are not written, `scratch` retains stale bytes, and **this gate fails**. Rev 1 had a truth table test over synthetic inputs that could never see a wrong flag bit |
| **G3a** | **Floor negative**: A/A contrast (same code both legs) ⇒ `NotResolved` | shrink the floor to a quantum → `Resolved` appears → gate fails |
| **G3b** | **Floor POSITIVE**: contrast between a calibrated spin of K and 3K ticks ⇒ `Resolved`, with `median_delta` within tolerance of 2K | `fn resolve(..) -> NotResolved{..}` (a stub that passed every rev-1 gate) fails here |
| **G4** | **Overflow observability**: fill a lane past capacity ⇒ `overflow > 0` **and** the artifact names it | silence the overflow path → the artifact assertion fails |
| **G5** | **Command census, two-sided**: disarmed ⇒ `profiling_cmds == 0`; armed ⇒ `timestamps >= 2` | record one profiling command on the disarmed path → first clause fails. (Golden pins are kept as a secondary pixel check with an explicit note that a BMP SHA-256 is structurally blind to this — rev 1 cited them as the proof) |
| **G6** | **Partition check**: a `PartitionGroup` containing a `TopOfPipe` member refuses to sum | declare a TOP zone into `PartitionGroup::VbRun` → reporter prints `sum=NOT_VALID`; the test asserts it does |
| **G7** | **Unclaimed refusal**: emit from an unclaimed `std::thread` ⇒ `unclaimed_drops > 0` **and** no lane cursor moved | route unclaimed threads to lane 0 → the second assertion fails |
| **G8** | **Concurrency computability**: a two-system schedule with a known conflict and a known-compatible pair ⇒ `declared` matches the conflict graph and `observed` is non-zero for the compatible pair | discard intervals at drain → `observed` is unavailable → gate fails (rev 1's design failed this by construction) |
| **G9** | **Instrument disclosure**: `FrameRecord.instrument > 0` when armed and `run_net < run_gross` | omit the instrument accounting → gate fails |
| **G10** | **Dual-record agreement** (rung 5-6 only): old and new collectors agree within one quantum on the same frame | a port that shifts a bracket fails before the old one is deleted |

### Unit tests

SPSC ring empty/full/wrap/`u32`-cursor-wrap · `Sample` layout and flag round-trip · `ZoneLane` = 128 B with halves on distinct lines · `SystemMeta` = 256 B (const-assert + test) · concurrent first-execution mints **one dense id** across 16 threads with **no leaked counter value** (m3) · registry exhaustion terminal, 90 % warning fires exactly once · the 2×2 label truth table, all four rows · `dur` saturation emits an `Extension` sample carrying the high bits · `counter(id)` returns `None` for a `Span` zone (no panic) · `resolve` is `NotResolved` at exact equality with the floor · ABBA leg order is `A B B A` · leg summaries survive a window wrap · frame attribution: a sample straddling a boundary lands in the frame containing its `begin` · a sample older than the window increments `late` · sealing with `GpuPass` disarmed · slot retires on `fence_seen && grace == 0` with an unwritten pair · `VK_NOT_READY` maps to `Ok` with clear availability (Mock) · `Floor` cannot be constructed from a null-zone window (compile-fail test).

### Property tests

For any interleaving of `n` pushes and `m` drains: `pushed == drained + in_ring + overflowed` · median/p95 match a sorted oracle over random windows · the overlap matrix is symmetric and reflexive · a `BottomOfPipe`-only partition group's member sum equals its run bracket within one quantum for any ordering · frame attribution is a total function (every drained sample lands in exactly one frame or one drop counter).

### Loom / Miri

Loom: one lane, 1P/1C, capacity 2, 4 ops — no lost sample, no double-drain, no read of an unpublished slot; plus the `seal`/`marks` publish. (Loom **release** binaries crash at startup on this box — run debug.)
Miri under Tree Borrows: `unsafe impl Sync for ZoneLane`, the raw sample write through the published pointer, `FrameSlot.marks` `UnsafeCell` access.

### Benchmarks (criterion, `harness = false`)

`zone_cost` (three legs: on / off-mask / off-feature) · `drain_cost` (400 samples) · `window_reduce` (1024 zones × 120) · `overlap_pairs` · `gpu_zone_retire`. Regression gate: `zone_cost` fails at +25 % over the committed baseline. Protocol per `docs/BENCHMARKING.md`: median-of-N, High priority, all-core affinity, **never two bench jobs concurrently** (hard project rule; `target/` once reached 74 GB and took the disk to zero, masquerading as mingw errors).

**Naming:** no binary may contain `time` / `update` / `setup` / `install` / `patch` (Windows os-error-740). Hence `zone_cost`, `drain_cost`, `gpu_zone_retire`, `contrast_floor`.

### `debug_assert!` invariants

`lane < LANE_COUNT` · `zone < MAX_ZONES` · `OPEN_DEPTH == 0` at drain (debug-only TLS, D3a) · `write - read <= LANE_CAPACITY` · `used_pairs <= MAX_GPU_PAIRS` · `pool.count == 2 * MAX_GPU_PAIRS` at reset (the width guard `gpu_timing.rs:492` already carries) · `!is_in_system_run()` at arm/drain (same-thread assertion only) · kind matches the accessor · `frames[i].state != Pending` before the reporter reads.

**Release-live** (the GPU path inherits the driver's release profile, `gpu_scene/mod.rs:7498`): the label computation, the retire deadline, every drop counter, the census clauses, and the `NOT RESOLVED` verdict.

---

## Answers to the review's open questions

1. **128-pair witness representation:** a `[u8; MAX_GPU_PAIRS]` mark array in `UnsafeCell` plus a single `AtomicU32 seal` (D5). Recorder cost per bracket is one plain byte store; the ordering cost is one `Release` store **per frame**, not per pass.
2. **Which control does `resolve` take:** neither of rev 1's two. It takes a `Floor`, constructible only from a cross-process session file or an in-sitting A/A control of the **same workload** (D11). The null zones are the *quantum*, printed separately, and cannot become a `Floor`.
3. **Which thread runs retire:** the dispatcher, via a `requires_dispatcher` system (A3).
4. **Is `profiling` default:** yes, the cargo feature is default-on so shipped code carries the sites; `CHANNEL_MASK == 0` by default, so the runtime cost is one predicted-not-taken branch and the drain is `if mask == 0 { return }`. The feature-off leg exists and is CI-built. `SystemMeta.zone` is unconditional, so the 256 B pin is configuration-independent (D1/M6).
5. **Frame id of a sample:** not carried. Attribution is a merge walk of the lane's TSC-monotone `begin` against `frame_begin_tsc[]`, and the drain stops at the live frame's cut (A2). Late arrivals older than the window are counted (`W9209`).
6. **Test-target migration:** enumerated per rung above; the deletions all happen in rung 7 together with every consumer.
7. **`MAX_ZONES`:** closed — 1024, with `big_zone_table` → 4096 and a 90 % warning tier. It was the wrong question to leave open.

---

## Open questions (remaining)

1. **`profiling-alloc` shim.** A global allocator is process-wide and perturbs everything it measures; the 19 zero-alloc gates answer "allocations per frame" more precisely, in test binaries, without perturbation. **Recommendation: build it, default off, artifact-labelled as a diagnostic mode whose numbers are not comparable to an unarmed run.**
2. **Artifact granularity.** `append_artifact` writes one `[[window]]` table per report, so a killed run leaves partial evidence (rev 1's write-at-exit lost exactly the runs most worth diagnosing). Open: whether a *frame-level* stream is ever wanted, which would make the artifact a stream rather than a document.
3. **v1.1 calibrated timestamps.** Deferred by D14. The trigger to revisit is a concrete cross-domain question — "is the CPU recording the frame, or waiting for the GPU to finish it?" That question is real and will come. Revisit after rung 8, with the question named first.
4. **`Immediate` support on this box** is unproven; the design probes and records the resolved mode rather than assuming, so the Frame channel may remain FIFO-bounded here. If it is unsupported, the frame-channel wall clock stays labelled non-decidable and rung 8's present-mode work reduces to the labelling.

---

## Checklist

Structure ✅ · Data structures ✅ (`repr`/align on every shared type; hot/cold split — `ZoneDesc` is `&'static` and never on the emission path; sizes computed and summed; false-sharing padding pinned by `const _`) · API ✅ (no `dyn`; no internal type in a signature; explicit lifetimes; kind-typed windows; no bare-delta constructor; no panicking accessor) · Multithreading ✅ (per-datum table; every ordering justified; no `SeqCst`; partition = lane; seven race-freedom clauses; `Send`/`Sync` including `ZoneGuard: !Send`; **no teardown**) · Correctness ✅ (cursor wrap, `dur` saturation with a lossless extension, dense minting under contention, GPU deadline, frame attribution, sealing, forgotten guard, panic unwind, multi-world, host-reset absence) · Integration ✅ (17 files, 4 new module groups, `Arena`/`ComponentPool`/`UnitId` untouched with a reason, 9 rungs each compiling alone with the subtractive one isolated) · Validation ✅ (13 showable-RED gates incl. two positive controls, 18 unit tests, 5 property tests, loom, Miri, 5 benches with a regression threshold, release-live list).

**N/A:** SIMD on the emission path — a 16 B store is one instruction; vectorisation belongs to A4 and is specified there.

---

## Findings disposition

| # | Finding | Disposition | Where |
|---|---|---|---|
| **B1** | `AtomicU128` does not exist | **FOLDED** — mark array `[u8; MAX_GPU_PAIRS]` + one `AtomicU32 seal` as the release edge; one Release store per frame, not per pass | D5, `FrameSlot`, ordering table |
| **B2** | `WITH_AVAILABILITY_BIT` is `0x4`, not `0x20`; no gate could catch it | **FOLDED** — constant corrected against `ffi.rs:846,849`; **G2c** availability truth control (before-submit ⇒ 0, after-fence ⇒ 1, never-written ⇒ 0) added, which fails on a wrong flag bit | D4, G2c |
| **B3** | Floor: wrong instrument, wrong scope, wrong sigma; `Resolved` on noise | **FOLDED** — floor is the tree's protocol (3σ × CV of the workload, 7 processes × 3 repeats); null zones demoted to *quantum*; `Floor` type with no quantum constructor; `resolve` takes `Floor`, unifying it with G3's A/A control; `DEFAULT_REPEATS` re-read correctly | D11, `Floor`, A6, G3a |
| **B4** | G5 vacuous — PINS are BMP SHA-256, blind to commands | **FOLDED** — `RecordCensus` at the `vkCmd*` sites, two-sided gate; goldens demoted to a secondary pixel check with the blindness stated | D17, G5 |
| **B5** | No positive control for `Resolved`; a stub passes | **FOLDED** — **G3b** calibrated K vs 3K spin must return `Resolved`; also G2b positive label control | G3b, G2b |
| **B6** | `concurrency()` uncomputable from the store | **FOLDED** — declared = static `compat` matrix from `ConflictGraph`; observed = retained `intervals` ring (8 frames × 512 systems, 64 KiB); G8 gates computability | D9, `Profiler`, G8 |
| **B7** | UAF on foreign lanes at disarm; `is_in_system_run()` cannot see other threads | **FOLDED** — **no free ever**: slab allocated at first arm, retained for process life; disarm is a mask store; the honest resident-cost consequence stated | D15, race clause 5 |
| **B8** | PR-5/PR-6 don't compile; G2's RED unshowable; test files unlisted | **FOLDED** — additive dual-record rungs 5-6, single subtractive rung 7 migrating all 15 named consumers in one commit; G2 split into structural G2a (no blocking reader, non-hanging red) + G2b positive label control | Rung table, G2a/G2b/G10 |
| **M1** | Frame attribution undefined; sealing undefined with GPU off | **FOLDED** — merge-walk attribution against `frame_begin_tsc[]`, drain stops at the live-frame cut, `late` drop counter; sealing rule covers the GPU-disarmed case and the `Partial` deadline | A2 |
| **M2** | `depth` unincremented, and UB as a plain field in a `static` | **FOLDED** — `depth` removed from the lane; debug-only TLS `OPEN_DEPTH`; `capacity` becomes a `const`; no plain mutable field survives in `LANES` | D3a, `ZoneLaneWriter` |
| **M3** | Two registries, no sync rule, unwritable as typed | **FOLDED** — one `[AtomicPtr<ZoneDesc>; MAX_ZONES]`; the `descs` mirror deleted; desc-before-id publication order stated | D6, `Profiler` |
| **M4** | D2's premise false — no present/asset thread; the real hazard is UNATTACHED→lane 0 | **FOLDED** — taxonomy rewritten to the actual topology (68 lanes: workers/dispatcher/host/2 spare), resolution order defined, dual identity of one OS thread justified | D2 |
| **M5** | Retire's thread specified twice, incompatibly | **FOLDED** — pinned to the dispatcher via `requires_dispatcher` | A3, ownership table |
| **M6** | Feature default unstated; "disabled" names two costs; drain uncosted | **FOLDED** — feature default-on, mask default-0; drain added to the target table with an armed and a disarmed figure; `SystemMeta.zone` unconditional so the pin is config-independent | D1, target table, invariant 2, answer 4 |
| **M7** | The instrument sits inside its own primary number | **FOLDED** — drain moved to `App::update` before `Schedule::run`; `run_gross` / `instrument` / `run_net` all printed; G9 gates it | D16, G9 |
| **M8** | `WaveRecord.members` truncates above 256 systems | **FOLDED** — membership mask deleted; `RoundRecord` keeps dispatch shape only; declared concurrency comes from the static matrix | D9 |
| **M9** | `rate_per_frame` panics — the kind distinction is not type-enforced | **FOLDED** — kind-specific `SpanWindow`/`CounterWindow`/`GaugeWindow`; wrong kind ⇒ `None` | D13, API |
| **M10** | `p10/p90` estimator undefined; control asymmetric | **FOLDED** — paired per-round leg-median deltas, quantiles over rounds; both `Contrast` arms carry identical fields incl. `control_cv` and `order_bias` | D11, `Contrast` |
| **M11** | No leg mechanism, no retention; windows overwritten | **FOLDED** — `LegSummary` arena (8 legs × 16 subscribed zones), `seal_leg` at boundaries, `resolve` consumes summaries; the caller applies the A/B configuration | A5, `ContrastPlan` |
| **M12** | Layout fork asserted, not decided | **FOLDED** — frame-major decided with the line arithmetic (≤256 vs ~1600 lines/frame) | D8 |
| **M13** | `calib_residual` = worst-of-N contradicts the peak-to-peak rejection; 50 ms hitch unstated | **FOLDED** — `calib_cv` + `calib_rejected`; window bounded to 20 ms and stated as a setup hitch | D3 |
| **M14** | `hostQueryReset` is a disarmed-path change; fallback unspecified and unavailable here | **FOLDED** — enablement is a `pNext` bit recording no commands (stated); fallback fully specified as `needs_cmd_reset` + frame-top `vkCmdResetQueryPool`, with `GPU_RING_DEPTH=4` proving it never stalls | D18 |
| **m1** | `total` sizing 4× off | **FOLDED** — recomputed; `total` widened to `u64` (also carries counter levels), full table ≈ 2.23 MiB | `Profiler` |
| **m2** | `dur` saturation detectable but not recorded | **FOLDED** — a `#[cold]` `Extension` sample carries the high bits; lossless | `Sample`, A1 step 6 |
| **m3** | Concurrent minting leaks ids | **FOLDED** — CAS to `RESERVED` first, then `fetch_add`; losers spin; ids stay dense; unit test added | D6 |
| **m4** | "no cold branch at all" is wrong | **FOLDED** — reworded: the branch is emitted, statically predicted not-taken, never taken | D6 |
| **m5** | 128 pairs exactly fills the witness — the same fixed-width wall | **FOLDED** — byte marks scale to any pair count; the only bound is the array size, const-asserted against pool width | D5 |
| **m6** | `min_max` per-sample vs `median` per-frame-sum, one type | **FOLDED** — `median_frame_ticks` vs `per_sample_min_max`, distinct names on `SpanWindow` | API |
| **m7** | p95 at n=120 is a 6th-order statistic | **FOLDED** — `p95` returned as `(p95, lo, hi)` neighbouring order statistics, with `n` | API, A4 |
| **m8** | `FrameRecord` undefined; `counter_level` column absent | **FOLDED** — `FrameRecord` defined (72 B); counter levels live in the `total` column, no separate array | Data structures |
| **m9** | `VK_NOT_READY` mapping unstated | **FOLDED** — `Ok(())` with availability bits clear, pinned by a Mock test | D4, unit tests |
| **m10** | Artifact-at-exit loses partial evidence | **FOLDED** — `append_artifact` writes one `[[window]]` table per report | API, open q2 |
| **m11** | `Immediate` support assumed | **FOLDED** — probed via the existing `present_mode_supported`, resolved mode recorded, loud fallback | D12, open q4 |
| **m12** | `SystemMeta` arithmetic 243 vs 244 | **FOLDED** — corrected to offset 242 / 244 total; a `const _` assert added beside the existing test | Invariant 2 |
| **m13** | Eight codes hard-wired to `eprintln!` with no seam | **FOLDED** — single `#[cold] emit_diag(code, fields)` seam; no emission-path printing (overflow is counted, reported at drain) | API, Integration |