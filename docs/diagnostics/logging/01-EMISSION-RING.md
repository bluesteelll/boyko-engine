# Logging — The Emission Path and the Per-Lane Ring

<!-- CONTRACT
provides: logging/emission-path
provides: logging/ring-and-statics
assumes:  substrate/clock-source
assumes:  substrate/lane-registry
assumes:  substrate/lane-write-sites
assumes:  substrate/loss-vocabulary
assumes:  substrate/never-freed-storage
assumes:  seam/build-axis
assumes:  seam/free-when-off
assumes:  logging/goal-and-audiences
assumes:  logging/budgets-and-invariants
-->

> **Carved from** `docs/LOGGING-SYSTEM-PLAN.md` (DRAFT v4) — Decisions 1, 1a, 2, 3, 4, 5, 8, 13, 14, 15 and 21; the `level.rs` / `site.rs` / `record.rs` / `lane.rs` / `target.rs` blocks of §Data structures; the emission slice of §Public API; §Algorithms A and B; and §Multithreading model **whole**. Diff against that document until it is retired. Content is **carried**, not summarised; the only edits are the S13 re-cuts named inline and the two internal divergences recorded at the end.

---

## Decision 1: Deferred formatting — POD payload + `&'static LogSite` + monomorphised decoder

**What.** The call site writes a 20-byte packed header containing a `*const LogSite` and a POD encoding of the arguments. Formatting happens on the sink via `site.decode: unsafe fn(*const u8, usize, &mut LogFormatter)`, monomorphised per *argument-tuple type* (Rust dedups identical monomorphisations, giving Quill's `log_statement` sharing for free).

**Why.** spdlog is asynchronous and still costs 242 ns median because it formats on the caller; Quill/NanoLog cost 7-9 ns because they do not. `core::fmt::Arguments<'a>` borrows its temporaries and cannot outlive the call, so the C++ varargs-capture trick has no Rust analogue.

**Alternatives rejected.** *defmt linker-section interning* — depends on ELF section semantics and a linker script; this toolchain is windows-gnu / PE-COFF, and an 8-byte `&'static` pointer is already free and portable. *`tracing`* — `event!` emits a static callsite, an `Interest` atomic load and `__is_enabled()` even when disabled, then dispatches through `Box<dyn Layer>`; Bevy's own docs concede the runtime-filter cost. *Caller-side format into a stack buffer* — 100+ ns and re-imports `core::fmt` codegen into 83+ sites.

**Trade-off.** Arguments must be `LogValue` (POD + `&str`). A `Display` is rendered by `dsp!` at the call site, and that cost is **visible in the source**, which is the point.

---

## Decision 1a: Record length is a runtime quantity — `encoded_len(&self)`, not `const ENCODED_LEN` *(fixes B2)*

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

---

## Decision 2: Three gates, `&&`-chained — two const-folded, one byte load

```rust
if T::STATIC_CEILING as u8 >= LVL as u8            // const: per-target compile ceiling
    && $crate::GLOBAL_CEILING as u8 >= LVL as u8   // const IN boyko_log (see N27)
    && $crate::runtime_ceiling(T::ID) >= LVL as u8 // Relaxed u8 load
{ $crate::emit_impl(...) }
```

**Why.** The `&&` short-circuit is what guarantees arguments are never evaluated — `log`'s exact shape, and why its docs warn against side-effecting arguments. Unreal explicitly does *not* give this guarantee and documents `UE_LOG_ACTIVE` as the workaround. The per-target compile ceiling is Unreal's two-verbosity model, which `log`/`tracing` lack. The runtime gate is one `Relaxed` load from `static CONTROL: [AtomicU8; 256]` plus one `and` — puffin's measured ~1 ns `AtomicBool` shape, generalised (Decision 14).

**Alternatives rejected.** *Cargo features for the ceiling* — features are additive and unified; one crate enabling `max_level_trace` re-enables it for everyone. `option_env!` in a `const fn` has no such failure mode.

**Where the const comes from, after S9.** `GLOBAL_CEILING` is derived from **one** env var, `BOYKO_PROFILE`, read by **exactly one `build.rs` in the workspace — `crates/boyko_diag/build.rs`**, which sits at the bottom of the graph so a change to it rebuilds every dependent. `crates/boyko_log/build.rs` is **not created** (v3 planned one; S9 deletes it), and neither is `crates/boyko_ecs/build.rs`. Staleness is closed by `boyko_diag/build.rs` emitting `cargo:rerun-if-env-changed=BOYKO_PROFILE`. `boyko_log` re-exports the value as its own `pub const GLOBAL_CEILING`, so **N27's property is preserved verbatim**: the macro still writes `$crate::GLOBAL_CEILING`, the `option_env!`/`env!` is still never expanded into a caller crate where no rerun directive exists, and the const still folds at every site. The axis itself, its profile→const table and its CI-leg count are `seam/build-axis`'s.

`BOYKO_LOG_MAX_LEVEL` survives **only** under `BOYKO_PROFILE=custom`; setting it while a named profile is selected is a `compile_error!` with a named message. That is the one axis rule S9 exists to enforce — two axes is how a binary ends up printing a ceiling its profile does not name.

**Trade-off.** Changing the profile rebuilds the workspace. `MAX_TARGETS = 256` is a hard cap.

**What the two kinds of gate can and cannot buy** *(S13, folded in at the split)*. Gates (a) and (b) are the **COMPILE-TIME CEILING**: a `const false` short-circuits the `&&`, and the arm **and its operands** are deleted — no branch, no symbol, no argument evaluation. Gate (c) is the **RUNTIME FLAG**, and it is the site's floor: **one `.bss` byte load plus one branch the predictor always gets right, at every surviving site, in every frame, forever.** A runtime flag has to be read in order to be a flag, so **gate (c) cannot be driven to zero by turning the flag off** — only the compile ceiling removes it. This is why the design keeps two axes rather than one, and why a *shipped* binary can still be asked for a log while a compile-only design cannot. The full two-axis cost table, the enable-path move and gate `GJ1`'s control leg live once, in `seam/free-when-off`.

---

## Decision 3: Statics in `.bss`; `Off == 0`; a genuinely-off build *(extends v1, fixes M21; re-specified by F4, F21, F25; lane extent re-sourced by S3/S9; boot/enable split re-cut by S13)*

**What.** `LANE_BYTES = 16 KiB`. The **lane count is no longer this crate's constant**: it is `boyko_diag::LANE_COUNT`, **80 in every build profile — Q1 RESOLVED, the profile axis is deleted** (`substrate/lane-registry`). v3's `MAX_LANES = 128` is deleted. 80 is a *max*, not a sum: 64 workers (a hard const matching `thread_pool.rs:49`) + dispatcher + host + 14 claimable spares, which is ~7× the measured non-pool thread count in this engine. The lane array is a static, sized by that `const`:

```rust
pub const LANE_ARRAY_LEN: usize =
    if (GLOBAL_CEILING as u8) == 0 { 0 } else { boyko_diag::LANE_COUNT as usize };
static LOG_LANES: [LogLane; LANE_ARRAY_LEN] = [LogLane::NEW; LANE_ARRAY_LEN];
```

> **BLOCKER RESOLVED — and it cost this file 792 KiB it had not been carrying.** `LANE_COUNT = 32` in `shipping` was unsound against a worker-anchored topology: 64 workers alone need 64 indices and the floor is 66. **Q1's answer was to delete the profile axis, not to pick a number** — `LANE_COUNT = 80` everywhere (`substrate/lane-registry`). This file consumed the 32 in two tables. The corrected rows are below and the difference is stated where it lands rather than folded away: `LOG_LANES` 512 KiB → **1.25 MiB** (+768 KiB) and `SAMPLE_CTR` 16 KiB → **40 KiB** (+24 KiB), so the shipping reserved total goes 1 220.26 KiB → **2 012.26 KiB ≈ 1.97 MiB**. Both are `.bss` reserved extents whose resident cost is `claimed_lanes × row`, so **the resident change is bounded by how many lanes a shipped title actually claims, not by the constant** — which is the property that made 80-everywhere affordable in the first place. The joint consequence, which is larger and is **not** confined to reserved extents, is `seam/joint-cost`'s.

**S13 re-cut of the boot line.** v4 wrote: *"`boot()` is a no-op when `GLOBAL_CEILING == Off`: no sink thread, no panic hook, no `RATE` traffic."* That is now the **weaker** half of a stronger rule. **`boot()` spawns nothing and installs nothing in ANY profile**: it is a pure struct-fill. The sink thread, the process-global panic hook and the `PRE_FLUSH` registration move to **`boyko_log::enable()`**, which runs at launch, before the game loop, on the host thread. With the flag off, none of them exists — in `dev` as much as in `off`. The `GLOBAL_CEILING == Off` case remains the *stronger* statement, because there is then nothing to enable and no lane array to enable it into. The lifecycle text this changes is `logging/sink-lifecycle`'s (Decisions 10 and 12); the argument for the move is `seam/free-when-off`'s.

**What the smaller array costs, stated.** The claim scan no longer spreads by `hash(thread_id)`: `boyko_diag::claim_lane()` is a load-then-CAS over the 14 spares in index order, so concurrent claimants can convoy on the first free slot — bounded at 14 CAS attempts on a `#[cold]` path taken **once per thread**. A thread that never calls `release_lane()` holds its spare for the process: bounded at 14 × `LANE_BYTES` = 224 KiB, counted as `lanes_leaked` and printed in the census.

**Why — four properties, each load-bearing.**

1. No boot check on the hot path; a heap block behind an `AtomicPtr` would cost an `Acquire` load plus a null branch per record and create a "not booted" state at every site.
2. The unbooted default is correct and free: `.bss` is zero, `Level::Off == 0`, so every target reads `Off`, the gate fails, nothing happens. **This is the property S13 generalises**: the *un-enabled* default is correct and free for the same reason and by the same mechanism, and the profiler's `ARM_MASK == 0` is the same trick one crate over.
3. Demand-zero paging: **with the flag ON**, resident cost is `claimed_lanes × 16 KiB`, typically 8-12 lanes ≈ 128-192 KiB. **With the flag OFF, `claimed_lanes == 0` and resident is 0** — no lane is claimed, no buffer is touched, and an untouched `.bss` page costs address space rather than physical RAM *(S13)*. The limit on that second sentence is stated where the gate lives: `substrate/section-report` proves the bytes are absent from the **image**; that the loader leaves the pages uncommitted is **UNPROVEN and is not claimed**.
4. **`BOYKO_PROFILE=off` is a real off switch**, not merely a site-folding switch: zero lanes, zero threads, zero hooks. *(v3 named `BOYKO_LOG_MAX_LEVEL=off`; S9 makes `BOYKO_PROFILE` the single axis and that spelling survives only under `custom`.)*

**Gated, not assumed** (M21): `.bss` residency of a `MaybeUninit` static on PE/COFF is a toolchain behaviour, so gate **G3** asserts the section owning `LOG_LANES` carries a size with no raw data. The probe itself is **`boyko_diag::section_report`** (S12) — one implementation shared with the profiling plan's G22a/G22b, so a PE/COFF toolchain change reds one gate instead of splitting two. **`llvm-tools` is not installed on this machine** (measured), and `substrate/section-report` makes tool absence a **RED, never a SKIP**.

**G2 is re-specified: each of its three legs names its observation mechanism** *(fixes F4)*. v2's G2 asserted "`size_of_val(&LANES) == 0` and that no sink thread is spawned" and Decision 3 added "no panic hook", with no mechanism for either non-size leg — so `boot()` could spawn the thread and install the hook while `LANE_ARRAY_LEN` was 0 and G2 would still be green.

| Leg | Mechanism | Named red state |
|---|---|---|
| (a) size | `const _: () = assert!(LANE_ARRAY_LEN == 0)` in the off leg. **This is a const tautology and is kept only as env-plumbing proof** — it reds when `BOYKO_PROFILE=off` fails to reach the crate **through `boyko_diag`'s `build.rs`**, which is now a two-crate path and therefore a *more* useful plumbing check than v3's one-crate version. It is annotated as such in the test so nobody mistakes it for the claim | unset the CI leg's env var |
| (b) no sink thread | **OS thread count across `boot()`**: Windows `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)` counting threads whose `th32OwnerProcessID == GetCurrentProcessId()`; Linux `std::fs::read_dir("/proc/self/task").count()`. Test-only, ~15 lines, one `#[cfg]` pair. **Its own control**: the same fixture spawns one deliberate `std::thread` and asserts the probe's count rises by exactly 1 — so a probe that always returns a constant reds before it can certify anything. **S13 adds a second subject**: the same probe runs across a **flag-off run of a `dev` build**, not only across an `off`-profile build, because "no thread while the flag is off" is a claim about the run and not about the ceiling | make `boot()` spawn unconditionally ⇒ count rises ⇒ leg (b) reds. **S13's additional red**: move the sink-thread spawn back from `enable()` into `boot()` ⇒ the flag-off run gains a thread ⇒ leg (b) reds |
| (c) no panic hook | **Behavioural, not identity-based** — `std::panic::take_hook()` is destructive and returns an unidentifiable `Box<dyn Fn>`. The fixture installs its **own** probe hook before `boot()`, then panics under `catch_unwind`, and asserts (i) the probe fired **exactly once** and (ii) the captured stderr contains **no** `boyko-log` marker line. **S13 re-points this leg the same way as (b)**: the subject is the flag-off run | make `boot()` chain its hook unconditionally ⇒ the marker appears ⇒ leg (c) reds. **S13's additional red**: move the hook install back into `boot()` ⇒ the flag-off run gains a hook ⇒ leg (c) reds |

**What G2 cannot claim**: that the off build has *no* cost. It has one: the crate is still compiled, linked and depended upon by every other crate. G2 bounds the *runtime* footprint to zero lanes, zero threads and zero hooks; it says nothing about compile time or binary size, and the `shipping` profile, not `off`, is the configuration a real title ships. **And after S13 it still cannot claim the per-site cost is zero on a flag-off run** — that is gate (c) of Decision 2, it is ≤ 3 ns, and only the compile ceiling deletes it.

**`.bss` budget matrix, stated in full** *(fixes F25 — v2 left `STAGE_BYTES`'s backing store unspecified, and "no `Vec`/`Box` in any signature" is carefully narrower than the Principle-1 claim a reader takes from it)*. Every one of these is a `.bss` static, demand-zero, never heap. **Every figure in this table is a RESERVED extent, not a resident cost** *(S13)* — reserved and resident coincide only for a table the process actually touches, and with the runtime flag off it touches none of them.

| Table | `dev` | `shipping` | Note |
|---|---|---|---|
| `LOG_LANES` | **80** × 16 KiB = **1.25 MiB** | **80** × 16 KiB = **1.25 MiB** | reserved; resident is `claimed_lanes × 16 KiB`, and **0 with the flag off**. **80 in both columns from `boyko_diag::LANE_COUNT` — Q1 deleted the profile axis**; the shipping cell read 512 KiB at 32 lanes until that resolution propagated |
| `RATE` | 512 × 64 B = 32 KiB | same | per-code; `Once`/`OnceCounted` no longer use it at all (Decision 8) |
| `SAMPLE_CTR` | `LANE_COUNT` × 256 × 2 B = **40 KiB** | **40 KiB** | one row per lane, producer-private (v3: 64 KiB at 128 lanes). Both columns move together with `LANE_COUNT`; the shipping cell read 16 KiB at 32 lanes |
| `TARGET_STATS` | 256 × 64 B = 16 KiB | same | consumer-written |
| `CONTROL` + `TARGETS` + `DYN_NAMES` | 256 B + 2 KiB + 2 KiB | same | |
| `ONCE_SITES` list heads | 8 B | same | one `AtomicPtr`; the nodes are the per-site statics themselves (M1) |
| `STAGE` | 256 KiB | 256 KiB | **`static STAGE: UnsafeCell<[u8; STAGE_BYTES]>`** — consumer-owned, reused every drain, never allocated |
| **`ECS_HANDOFF`** | **256 KiB** | **64 KiB** | *(new — B2)* the sink→ECS SPSC byte ring; present only when `ecs_ring` is enabled. Absent in `shipping-min` (no ECS reader) |
| `SITE_DICT` + `SINK_OUT` | 64 KiB + 1 MiB | 64 KiB + 256 KiB | binary sink only; absent unless `BinarySink` is configured |
| **Total reserved** | **≈ 2.90 MiB** (2 972 KiB) | **≈ 1.97 MiB** (2 012 KiB) | resident is a small fraction of each **when armed**, and **0 when the flag is off** |

**The two totals, summed from the rows above so a reader can check them rather than trust them.**
`dev` = 1280 + 32 + 40 + 16 + 4.25 + 0.008 + 256 + 256 + 1088 = **2 972.26 KiB ≈ 2.90 MiB**.
`shipping` = 1280 + 32 + 40 + 16 + 4.25 + 0.008 + 256 + 64 + 320 = **2 012.26 KiB ≈ 1.97 MiB**

**The `dev` row did not move and the `shipping` row moved by 792 KiB, for one reason: Q1.** `dev`
was already at 80 lanes, so every `LANE_COUNT`-derived cell there was already correct; `shipping`
carried 32 in two cells (`LOG_LANES`, `SAMPLE_CTR`) and both had to follow the constant. The old
sum, `512 + 32 + 16 + … = 1 220.26 KiB`, is kept in this sentence rather than deleted so that a
reader meeting the older figure elsewhere in the corpus can tell **which** revision they are
holding — the pair that matters is (1 220.26 at 32 lanes) and (2 012.26 at 80).
(`CONTROL` + `TARGETS` + `DYN_NAMES` = 0.25 + 2 + 2 KiB; `ONCE_SITES` = 8 B).

> **The `shipping` figure is CORRECTED here, and the correction is stated rather than made quietly.**
> v4 printed **≈ 1.15 MiB (1 180 KiB)** for this column (`docs/LOGGING-SYSTEM-PLAN.md:235`, carried
> byte-identically into the carve). That is **40 KiB below the sum of its own rows**, and it is not a
> different configuration: the two optional rows are `ECS_HANDOFF` (64 KiB) and `SITE_DICT` +
> `SINK_OUT` (64 + 256 KiB), and **no subset of the table's rows sums to 1 180 KiB** — the reachable
> figures below the total are 1 156.26 (drop either 64 KiB row) and 964.26 (drop `SINK_OUT`). The
> `dev` column, summed the same way, reproduces its printed 2 972 KiB exactly, which is what
> identifies the `shipping` cell as the arithmetic error rather than the convention as ambiguous.
> **Consequence for the seam, stated because the owner sums THIS cell.** `seam/joint-cost` takes the
> logger's `shipping` half straight from this row (it names it "1 180 KiB `shipping`") and computes
> the joint retail as `0.89 + 1.15 = 2.04 MiB`. With the row corrected the same sum is
> `0.89 + 1.19 = 2.08 MiB` — **and Q1 then moved BOTH halves**, so the sum the owner now faces is **`1.18 + 1.97 = 3.15 MiB`**. `logging/dispositions`' owner-facing question separately carried 1.16
> (rev 3's figure). **The joint row is `seam/joint-cost`'s to land** — this file states its own half,
> shows the arithmetic that produced it, and does not restate the joint one.

**Joint footprint, because the isolated one is not what a frame pays — and it is NOT stated here.** The joint table has exactly one owner, `seam/joint-cost`, because a joint number restated in three files is how this corpus contradicted itself; this file states **only this crate's column**, above, and the owner sums it. What this file does owe that owner is the **lineage of the logging half**, since the joint `dev` figure was historically built from it: the seam record's earlier 9.33 MiB used a logging `.bss` of 2.63 MiB — v3's 3.40 MiB less S3's lane cut (128 → 80 lanes, −768 KiB) and `SAMPLE_CTR` (64 → 40 KiB, −24 KiB) — and that figure **predates `ECS_HANDOFF`**, which B2 adds in this revision at 256 KiB in `dev`. The 2.90 MiB above already contains that 256 KiB, so **9.33 + 0.25 double-counts it**; the halves must be re-added, not patched, which is what `seam/joint-cost` now does. *(This file previously printed the joint totals ≈ 9.58 MiB `dev` / ≈ 1.95 MiB retail. Both are withdrawn here: the first is the patched figure just described, and the second never equalled the sum of its own operands in any revision.)*

**Why the substrate is bought, since it is no longer defended by a byte count.** The shared substrate exists for **correctness** — one lane number, one clock epoch, a loss report that cannot itself be dropped — **not** for footprint, and **no rung of this plan is justified by a byte count**. *(The "saves 0.78 MiB in `dev`, 0 in `shipping`" figure this paragraph used to carry is **withdrawn and not replaced**. It is rev-3 arithmetic — `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md:43`'s dev row `6.65 | 3.46 | 10.11 | 9.33`, i.e. `10.11 − 9.33 = 0.78` — and rev 4 recut only the WITH-SUBSTRATE halves: the logger's `dev` went 2.68 → 2.90 and its retail 1.10 → 1.15, the profiler's 6.65 → 6.67 and 0.85 → 0.89. Rev 3's "logging alone" operand, 3.46 MiB, **was never recomputed and survives nowhere in this corpus**, so no rev-4 naive sum can be formed and the saving derived from one cannot either. The saving is **UNKNOWN at this revision**; `seam/joint-cost` owns that disposition and no substitute figure is invented here. The rule above never rested on the number, which is the point. `substrate/dedup-rationale` records the same correctness-not-footprint rationale.)* The retail figure against the profiling plan's "≤ 1 MiB retail" headline remains an owner-facing VALUES question, recorded once in `seam/open-owner-calls` — and the correction above makes it **larger**, not smaller.

**Honest floor when "on"**: the matrix above, one OS thread (except `shipping-min`), one process-global panic hook, one `VmReservation`-backed `LogRing` when the ECS seam is enabled, and a mandatory dependency edge from every crate onto `boyko_log` **and** transitively onto `boyko_diag`. That is the cost of the system existing; it is stated, not smoothed. **The floor when off** is the per-site branch of Decision 2 gate (c), the address space above, and nothing else.

**Off-build dead code** *(fixes F21; simplified by S3)*: v2 set `LANE_ARRAY_LEN = 0` while §Algorithms B scanned `start..start+MAX_LANES` and indexed `LANES[i]` — dead but panicking code that G2 could not distinguish. **After S3 there is no claim scan in this crate at all**: the index comes from `boyko_diag::lane()`, and the only array access is `LOG_LANES.get(id as usize)`, which yields `None` on a zero-length array and takes the exhaustion path. In the off build no call site survives the const gate anyway, including the `Warn`/`Error` fallback, because `GLOBAL_CEILING == Off` deletes every level.

---

## Decision 4: SPSC byte ring per lane — the ring is ours, the **identity is `boyko_diag`'s** *(re-cut by S3)*

**What.** `LOG_LANES[i]` is a single-producer/single-consumer byte ring, indexed by `boyko_diag::lane()`. **This crate no longer mints, claims, retires or reclaims lane identity.** v3's `MAX_LANES`, its `hash(thread_id) % MAX_LANES` claim scan, its `Lane::owner` CAS, its `RETIRING` protocol and its `Drop`-carrying TLS guard are all **deleted** and replaced by `boyko_diag::lane` (`substrate/lane-registry`, A2): `lane()`, `set_lane()`, `claim_lane()`, `release_lane()`.

**Why the identity had to move, and not merely be shared.** Two registries mean two lane numbers for one thread: the same worker is lane 5 to the profiler and lane 37 to the logger, and no reader can then place a log line inside the zone it happened in — the one joint question the two subsystems exist to answer becomes unanswerable **by construction**, not by a bug. Separately, v3's `Drop` guard was exactly the `thread_local!`-destructor mechanism the profiling plan deliberately refused, **and** it was the sole source of this plan's "≤ 1 allocation on a thread's first emit" row. Deleting it takes that row to **0**.

**Why the ring stays ours.** True SPSC is why the threadpool's *router* is not reused: `current_worker_id_or_dispatcher_lane()` maps every non-pool thread to lane 0. `boyko_diag::lane` fixes the router without touching the ring, and the ring's shape is a logging decision:

- The producer caches the opposite cursor. The one published measurement on this question found padding **alone** made a ring *slower* — both threads still read the opposite cursor every operation — and only opposite-cursor caching *plus* padding moved throughput from ~32 to ~440 M ops·s⁻¹. We do both and treat padding as a hypothesis with an ablation bench, matching this repo's own `reference-componentpool-cache-stagger` lesson.
- Records are POD with no `Drop` and the array is a `static` that never moves, so a retired-undrained lane leaks nothing — which is why reclaim can be lazy and consumer-driven.

**Retire/reclaim, restated against the new owner.** `boyko_diag::release_lane()` marks the substrate's slot `RETIRING`; the consumer, per drain, reads `boyko_diag::lane_state(i)` and calls `boyko_diag::reclaim(i)` only after observing `RETIRING && read == write` for `LOG_LANES[i]`. The ordering argument is unchanged — the producer's last write precedes `release_lane()`, and the reclaim follows a drain to `write` — but the **state now lives in one place**, so the profiler cannot hand the same index to a new thread while this crate still believes it is live.

**`load`-then-CAS survives, in `boyko_diag`** (M10): the claim path is `if slot.load(Relaxed) == FREE { try CAS }` over the 14 spares. An unconditional `compare_exchange` over the array takes every occupied slot's line exclusive — the exact defect this repo already fixed at `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs:36-51` ("load first, store once"; verified this session — `WARNED.load(Relaxed)` at `:41`, `WARNED.store(true, Relaxed)` at `:44`, inside a `#[cold] #[inline(never)]` helper). The `hash(thread_id)` spread is gone with the scan; its cost is bounded and stated in Decision 3.

**Alternatives rejected.** *Double-buffer + wholesale swap* (`EventBuffer::swap_and_flatten`) — needs a quiescence point that boot code, the present thread and a driver callback do not have. *One MPMC ring* — CAS on every push, reintroducing the contention the per-lane design removes by construction. *Keeping a private lane registry and mapping it to `boyko_diag`'s at read time* — a mapping table is a second source of truth for one datum, which is Decision 14's `LogFilter` defect in a new place.

---

## Decision 5: Overflow drops, counts, and reports — corrected arithmetic, `u64` counters, one aggregated report *(fixes F6, F24; extended by X4; X4 REVERSED by S8)*

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

**Counters accumulate in `u64`; they never saturate** *(X4 REVERSED by S8)*. v3 made `dropped`/`dropped_bytes` `AtomicU32` with a saturation guard, on the argument that "an 8-byte RMW is more expensive". That rejection does not survive: on x86-64 `lock xadd` costs the same at 4 and 8 bytes, **and the lane-owned cell needs no RMW at all**. v4 adopts `boyko_diag::loss` (`substrate/loss-vocabulary`, A3) wholesale:

- a per-lane `LossCell { count: u64, bytes: u64, _pad: [u8; 48] }` (64 B, cache-line-owned), written by the **lane owner** with plain `u64` load/store — single-writer, no lock prefix, the same argument this plan already makes for `SAMPLE_CTR`;
- the consumer folds into a `LossTotal { count: AtomicU64, bytes: AtomicU64 }` and clears with **`fetch_sub(observed)`, never `store(0)`** — a `store(0)` loses any increment landing between the consumer's load and its clear;
- the class is `boyko_diag::LossClass` (`Overflow` for an admission refusal, `Unclaimed` for E6, `Refused` for `TOO_LARGE`, `Sink` for an `ECS_HANDOFF` or callback loss, `Rotation` for E21).

> **Two carried-forward qualifications the substrate owns.** (1) `substrate/loss-vocabulary`'s **F5** states that a *plain* `u64` cell read across threads is UB regardless of what x86-64 does and that Miri reports it; the cells are therefore `AtomicU64` accessed `Relaxed` by the owner, which lowers to the identical `mov` pair with no lock prefix, so the performance argument above survives verbatim. (2) `fold_into` carries an **open BLOCKER (Q2)**: `fetch_sub(observed)` closes the *consumer* side of the lost-update window and **does not close the producer side** — an owner increment is `load;add;store`, and a consumer `fetch_sub` landing between that load and that store is overwritten. D0 ships `loss.rs` **without** `fold_into` until that architect call is made, and one gate (logging G11 = profiling G4b = DG5) serves both subsystems.

Two things fall out. `SATURATED` and its census token are **struck** — a `u64` at 66 M offers·s⁻¹ needs ~8 800 years to wrap, so there is no ceiling state left for a reader to mistake for a number, and a token the census could never let a reader *compare* stops existing. And with the counters in `boyko_diag`, **the report of a loss is a read of a counter, not a record that can itself be dropped** — the second-order defect (the profiler's drop report being dropped and counted as a *logger* drop) is removed by construction rather than mitigated. Gate **G11**'s subject changes accordingly.

**One aggregated drop report per drain, not one per lane** *(fixes F24)*. v2 emitted a synthetic `boyko-W0102` "per drain per lane": at 125 Hz × 128 lanes that is ~16 000 sink-generated records·s⁻¹ against a stated ~500 K·s⁻¹ formatting budget — 3 % of the budget spent by the drop reporter competing with the drops it reports, and unbudgeted. v4 emits **one** `W0102` per drain carrying `lanes_affected`, `records`, `bytes` and the `LossClass` breakdown: 125 records·s⁻¹, a fixed cost. Per-lane detail lives in the census, which is polled, not streamed.

**Lane-exhaustion fallback** (M26; **destination corrected by B9**): a thread with no lane does **not** silently drop `Warn`/`Error`. It falls back to the synchronous channel — `write_oracle_line`, bounded per `logging/sink-lifecycle`'s Decision 9c — for those two levels only, and counts `Info`/`Debug`/`Trace` as `LossClass::Unclaimed`. v3 left the promise inert in the shipped configurations, because `write_oracle_line` targeted stderr **unconditionally** and `shipping`/`shipping-min` configure no console sink. v4 fixes the destination, not the sentence: `write_oracle_line` fans out to **every configured synchronous destination**, which in `shipping`/`shipping-min` is the boot-opened crash file. The cost is paid only in the exhausted case, and a test harness that exhausts lanes therefore cannot lose a severe record **in any profile** — which is what v3 claimed and did not deliver.

**Why.** Blocking on `error!` inside a driver callback under a storm is a deadlock. Silent loss turns a logger into a source of false confidence — the exact class this campaign exists to kill.

**Alternatives rejected.** *Block-on-full* (spdlog's default) — a mutex by another name. *Overrun-oldest* — destroys the record that reported the cause in favour of the one that reported the consequence. **This rejection is what made v3's `shipping-min` structurally wrong** (B8): with no consumer, drop-newest fills the lanes with boot-time records within seconds and refuses everything up to the crash, so the profile whose only product is a crash log was guaranteed not to contain the crash. v4 does **not** answer that by switching to overrun-oldest — the argument above still holds — but by giving `shipping-min` a **real consumer** (`logging/sink-lifecycle`, Decisions 10 and 25). Drop-newest with a consumer is a bounded loss; drop-newest without one is a guarantee of the wrong contents.

---

## Decision 8: Rate policy is DECLARED on the code and APPLIED per site; `Once` is a site-local latch that degrades to a pure load *(fixes M11, M12, F11; extended by X3)*

**What.** Each `W`/`E` code declares `Every` / `Once` / `OnceCounted` / `EveryN(n)` / `MinIntervalMs(ms)` in the registry. The *mechanism* differs by policy, and that distinction is the fix for F11:

| Policy | State lives in | Scope | Steady-state cost |
|---|---|---|---|
| `Once` | a macro-generated `static FIRED: AtomicBool` **beside the call site's `LogSite`** | **per SITE** | one `Relaxed` load from a site-private line, not-taken branch |
| `OnceCounted` | the same site-local static + an `AtomicU32` | per SITE | one load; **one RMW per suppressed occurrence** — opt-in, cost stated at the declaration |
| `EveryN(n)`, `MinIntervalMs` | `static RATE: [RateSlot; MAX_CODES]`, dense `code_idx` | per CODE | one RMW per occurrence |
| `Every` | — | — | nothing |

**Why `Once` had to become per-site** *(F11)*. `RATE` is indexed by `code_idx`, so a code-scoped `Once` fires **once per code, not once per site**. Read against the tree: the migration routes `crates/boyko_rhi_vulkan/src/device.rs:3100`, `:3158` and `:3189` — three independent capability degradations — through **one** code `W2102` with `RatePolicy::Once`. A device lacking all three would report **one** and silently lose two, uncounted (and `Once` deliberately does not count). That defeats the migration's own stated purpose, which is `crates/boyko_app/src/host.rs`'s written argument that a RELEASE-build degrade-to-off must be observable.

*(All three device sites re-verified this session at exactly `:3100` (DDGI storage), `:3158` (shadow-denoise RG16) and `:3189` (SSAO à-trous R16) — each an `eprintln!` under `#[cfg(debug_assertions)]`, which is itself part of the argument: they vanish in release, which is precisely what `host.rs`'s counter-example objects to. **The `host.rs` citation is repaired**: v4 writes `:228-233`; the sentence "Emitted UNCONDITIONALLY (not `#[cfg(debug_assertions)]`): a RELEASE-build degrade-to-Off must be observable, else an owner requesting `BOYKO_AA=ssaa` on a device that fails the dims/VRAM probe silently gets no supersampling with zero explanation (spec B11)" is at **`:230-234`**. The argument is unchanged.)*

`W2202` has the same shape across `bindless.rs` and `mesh_geometry_table.rs`.

The resolution is not to mint three codes — a code names a *class of condition*, and three near-identical codes make `explain()` worthless — but to put the latch where the diagnostic value is: **the site**. The macro expands, next to the `&'static LogSite` it already emits, a `static FIRED: AtomicBool = AtomicBool::new(false)`. Cost: 1 byte of `.bss` per `Once` site. Benefit beyond correctness: the steady-state load hits a **site-private, never-contended** line instead of a shared `RATE` line, so this is strictly cheaper than v2 on both axes.

```
Once:  if FIRED.load(Relaxed) { return; }          // steady state: load only, private line
       if !FIRED.swap(true, Relaxed) { emit }      // exactly one RMW, ever, per site
```

**Why the no-store property matters.** The audit found five hand-rolled latches with two implementations, one wrong: `crates/boyko_render/src/render_path_config.rs:311-313` and `:335-337` execute `swap(true, Relaxed)` on a shared line **inside an `#[inline]` per-frame reader, every frame forever** once the divergence holds. v1's replacement kept a per-frame `suppressed.fetch_add` — the same defect wearing a policy name.

*(Both sites re-verified this session. `:311` is `&& !frozen.warned_ddgi.swap(true, core::sync::atomic::Ordering::Relaxed)` and `:335` the `warned_ssao` twin; the enclosing `effective_ssao_config` carries `#[inline]` at `:326`. The `&&` short-circuit means the `swap` runs **only** when the divergence holds — and then it runs **every frame**, because `swap` is executed for its return value and the guard above it never becomes false. The claim is exact.)*

**Suppression is reported as policy, and the absence of a count is itself printed** *(fixes F10)*. The review is right that v2 contradicted its own Goal: "loss is counted and reported; never silent" against "the `Once` count … is **not reported**", with the census reporting neither (`suppressed` is neither `records` nor `dropped`). v3 resolves it in three moves rather than by softening a sentence:

1. The Goal now says suppression is **not loss** and is reported **as policy** (`logging/goal-and-audiences`, functional bullet 3).
2. **The census prints one row per FIRED SITE, from a real observation** *(v3's per-code line is deleted by M1)*. v3 printed `rate=Once fired=1` per code. With the latch per-site and `RATE` unused by `Once`, **nothing aggregated per code** — no structure enumerated the `FIRED` statics and `TARGET_STATS` is per-target — so `fired=1` was a literal, and it was wrong in exactly the three-site `W2102` case F11 was raised for. v4 makes the enumeration exist:

   ```rust
   /// Intrusive, insert-only, allocation-free. The NODE is the per-site static
   /// the macro already expands; the list only adds a `next` pointer to it.
   #[repr(C)] pub(crate) struct OnceSite {
       site: &'static LogSite,
       suppressed: AtomicU32,      // OnceCounted only; 0 and unread for plain Once
       next: AtomicPtr<OnceSite>,  // CAS-pushed exactly once, on this site's single fire
   }
   static ONCE_SITES: AtomicPtr<OnceSite> = AtomicPtr::new(core::ptr::null_mut());
   // SAFETY: insert-only, never removed, never freed — every node is a `'static`.
   // A pusher publishes `site`/`suppressed` before the CAS that links `next`, so a
   // reader observing a non-null pointer via `Acquire` observes a complete node.
   // The list is walked only by the census, which tolerates a node appearing
   // between two walks; a site absent from the list is a site that never fired,
   // and that absence IS the datum.
   ```

   *(The SAFETY block above is carried from `docs/LOGGING-SYSTEM-PLAN.md:1318-1323` with **one word changed**: the monolith's clause reads "publishes `site`/`fired`", because the monolith also gives `OnceSite` a `fired: AtomicBool` field at `:1312`. This file does not, and the divergence is deliberate — see §Divergences found at the carve, item 3. The clause is what a reviewer of an `unsafe` intrusive list actually needs, and it is the only reason `AcqRel` on the push is not `Relaxed`; it is stated here, at the struct, rather than left implicit in the ordering table.)*

   The push is a `#[cold]` CAS loop executed **once per site per process**, on the same branch that already performs the single `FIRED.swap(true)` — so the steady-state path is still a pure `Relaxed` load from a site-private line and **nothing is added to the budgeted path**. The census walks the list (`Acquire` on `next`) and prints, per fired site:

   ```
   LOG-ONCE code=W2102 site=device.rs:3100 fired=1 suppressed=UNCOUNTED(by policy)
   LOG-ONCE code=W2102 site=device.rs:3158 fired=1 suppressed=UNCOUNTED(by policy)
   ```

   A site that never fired is simply **absent from the list**, and its absence is the datum. `OnceCounted` rows carry a real integer in `suppressed=`. `RateSlot::fired` is **deleted** — it was dead the moment `Once` stopped using `RATE` (M1).
3. A code whose suppressed count genuinely matters declares **`OnceCounted`** and pays one RMW per suppressed occurrence — at its own declaration site, visible in the registry, with the cost written in the row. The engine's own `W2102`/`W2202` use plain `Once`; a game is free to choose otherwise.

**`EveryN(n)` requires `n` to be a power of two** *(X3)*, enforced by `const _: () = assert!(n.is_power_of_two())` inside `codes!`, so the test is `count & (n-1)` instead of `count % n`. v2's arbitrary `n` mis-samples across the `u32` counter wrap (~12 h at 100 K·s⁻¹) — invisible in a 300-frame bench, wrong in a session. The fix is *also* cheaper: an `and` for a division. Strictly better on both axes.

**Layout.** `RateSlot` is 64 B, one per cache line — four unrelated codes sharing a line (v1's 16 B slot) is false sharing between subsystems that have nothing to do with each other. `MAX_CODES = 512` ⇒ 32 KiB, in the same `.bss` regime as `LANES`.

---

## Decision 13: `fmtv` is deleted; `dsp!` formats in argument position *(fixes B4, and B2's second half)*

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

---

## Decision 14: ONE byte per target owns level + sampling + sync routing; `LogFilter` is deleted *(fixes M14; extended by X1)*

v1 had `LogFilter { ceilings: [Level; 256], dirty: bool }` mirroring `CEILINGS`, synced by a hand-rolled flag — two sources of truth for one datum, with a public `set_target_level()` writing only one of them, so the next unrelated `dirty` flip would silently push a stale value over the live one. It also re-offended the "capability/state is not a bare bool" rule.

**Replacement.** `CEILINGS` is renamed **`CONTROL`** and its byte is packed:

```
bit  [0..2]  level      (Off | Error | Warn | Info | Debug | Trace)
bits [3..6]  sample shift k  (0 = every record; else deliver 1 in 2^k)   — Decision 20
bit  [7]     sync route      (format on the caller, write synchronously) — Decision 20
```

`CONTROL` is authoritative for all three. The UI, a console command and any system read and write it through `target_control(id)` / `set_target_control(id, ctl)` / `set_target_level(id, lvl)`, the last of which is a **CAS** so it preserves the sibling bit-fields. There is no ECS mirror, no `dirty`, and no sync system. Change detection is not needed because there is nothing to reconcile.

**Why one packed byte and not three arrays** *(X1)*. The three runtime knobs the game-facing audience needs are delivered **in the register the gate already loaded**: one `Relaxed` byte load, one `and`, one `cmp`. A parallel `SAMPLE_SHIFT` array would cost a second load and a second cache line on the *enabled* path. `.bss`-zero still means level `Off`, shift 0, sync off — the "unbooted is free and correct" property is untouched, and **after S13 it is also the un-enabled default**: `CONTROL` is the runtime flag word for logging, exactly as `ARM_MASK` is for profiling. This is gated, not assumed: `log_disabled_runtime` must stay **NOT RESOLVED** against the v2 single-level shape in the same sitting (G10d), and if it resolves, the packing is reverted rather than the target being raised.

---

## Decision 15: Target IDs are compile-time-unique, VALID BY CONSTRUCTION, and cut into three bands *(fixes M15, F15; extended by X2)*

v1 hand-assigned `id = $id:literal` per target with a boot collision check that could only fire if *both* colliders registered — and nothing forced registration, so an unregistered target still gated against `CONTROL[ID]` and never tripped.

**Band map** *(re-cut by X2)*:

| Band | IDs | Uniqueness proof |
|---|---|---|
| Engine | 0..=95 | one `targets! { … }` table; `const _: () = assert!(strictly_increasing)` ⇒ **collisions do not compile** |
| Downstream source | 96..=223 | `define_target!` + boot check `boyko-E0104` naming both colliders |
| **Dynamic** | 224..=255 | minted at runtime from a name (`logging/game-facing-surface`, Decision 18); the mint is the uniqueness proof |

Registry check 7 asserts every `LogTarget` impl in the workspace resolves to a table row or a `define_target!` expansion. The honour system is confined to code we do not own, and that is stated.

**`TargetId` is valid by construction; the `pub` field is removed** *(fixes F15)*. v2 declared `pub struct TargetId(pub u16)` against `MAX_TARGETS = 256` with the bound checked only under `debug_assert!`, and made `target_level`/`set_target_level` public. That is either a panic in a hot-path indexing operation or `get_unchecked` **UB reachable from safe public API in release**, and v2 stated neither. v3:

```rust
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)]
pub struct TargetId(u16);        // PRIVATE field — the invariant is `.0 < MAX_TARGETS`
```

The only constructors are (a) `targets!`, (b) `define_target!` — both `const` and both carrying `const _: () = assert!(id < MAX_TARGETS)` — and (c) `register_dynamic_target`, which returns ids from the dynamic band only. There is therefore **no representable out-of-range `TargetId`**, and `CONTROL.get_unchecked(id.0 as usize)` carries a SAFETY comment naming that closed constructor set as the invariant.

**`TargetId::INVALID` is deleted; absence is `Option<TargetId>`.** A public in-band sentinel that indexes an array is the same hazard in a nicer coat. `register_dynamic_target` returns `Option<TargetId>`; a game that has not yet registered stores `None` and cannot call `dyn_info!` at all. This removes v3-draft's `UNREGISTERED_DROPPED` counter, which existed only to count an unreachable state — a counter for an impossible event is a gate that cannot fire.

`boyko_utils::TypeIntern` is **not** usable here, and the reason is **one**, not two: `ID` must be a `const` for gate (a) to fold, and a runtime-minted intern id is not a `const`. *(v3 gave a second reason — "`boyko_utils` depends on `boyko_log`, not the reverse" — which S2 strikes: `boyko_utils` stays a zero-dep leaf and gains no edge in either direction; verified this session, its `[dependencies]` is empty. The conclusion is unchanged; one of its two supports was false.)* Recorded so the next reader does not re-derive it.

---

## Decision 21: The session-scale integer audit *(completed by M2 — v3's table omitted the field it had just created and every `BinarySink` quantity)*

A 300-frame bench cannot distinguish a correct counter from one that wraps in 65 seconds. Every integer is audited against an hours-long session. **Rows marked ✚ are the ones v3's "every integer was audited" sentence did not cover**, which is what made the claim unbacked precisely for the sink a shipping title runs for hours.

| Quantity | Width | Behaviour at the limit | Where |
|---|---|---|---|
| `LogLane::write` / `read` | `u32` byte cursors | **Wraps, correctly.** Every comparison is `wrapping_sub`, every index is `& MASK`, and `w − r ≤ LANE_BYTES ≪ 2³¹`, so the unsigned difference is unambiguous across a wrap. Wrap arrives in ~2.4 h at 500 KB·s⁻¹·lane | E17, test 19 |
| `dropped`, `dropped_bytes` | **`u64`** *(was `u32`+saturate; S8)* | Accumulate. ~8 800 years at 66 M·s⁻¹. The per-lane cell is a single-writer `u64`, folded into an `AtomicU64` with `fetch_sub(observed)` | E18, G11 |
| `RateSlot::count` | `u32` | Wraps; harmless **only because `EveryN(n)` is power-of-two** (X3) | Decision 8 |
| `LogStats.*`, `TargetStat.*` | `u64` | ~584 years at 1 G·s⁻¹ | — |
| `LogRing::head`, `arena_cursor` | `u32` | Wrap-correct; masked indices into fixed-capacity columns | test 20 |
| `LogRing::seq` | `u64` | The reader's cursor. Monotone, never wraps in any reachable session | Decision 26 (`logging/game-facing-surface`) |
| ✚ **`LogLine::seq_lo`** | **`u32`** | **Stores only the low half of `seq`, and the reconstruction rule is this**: for any line still in the ring, `seq = ring.seq − ((ring.seq_lo ⊖ line.seq_lo) as u32 as u64)`, where `⊖` is `wrapping_sub`. Unambiguous because the ring holds at most `LINE_CAP ≪ 2³¹` lines, so the low-half difference is always the true difference. **The high half is never stored and never needs to be**, which is why the ~2.4 h wrap at 500 K rec·s⁻¹ is not a truncation. `since(cursor)` with a `cursor` older than the oldest retained line starts at the oldest and reports the difference in `LogRingIter::skipped` | test 20 |
| ✚ **`BinaryRecord::tsc_delta`** | **`u32`** | Delta from the file's current **anchor**. A `u32` of raw ticks spans **1.4 s at 3 GHz**, so the sink re-emits an anchor record whenever the delta would exceed `u32::MAX` **or** every 1 s, whichever comes first — and unconditionally after a rotation. A missed anchor is a decode refusal, never a wrong timestamp | Decision 22, G12b |
| ✚ **`BinaryRecord::site_id`** | **`u16`** | Indexes `SITE_DICT`'s **4096** entries, so the width has 16× headroom over the table. The **table**, not the width, is the limit: on a full `SITE_DICT` the sink emits `boyko-W0116` once and writes an **inline site record** (file/line/fmt spelled out) instead of a dictionary reference, so no record is lost and no id is reused | Decision 22 |
| ✚ **`BinaryRecord::len` / `flags`** | `u16` / `u8` | `len ≤ MAX_RECORD_BYTES = 2048`, checked at runtime in every profile (E3); `flags` has 3 of 8 bits used | E3 |
| ✚ **File offset / rotation counter** | `u64` / `u8` | Offsets are `u64` (17 EB). `Rotation::keep` is `u8`, so at most 255 retained files; the rotation *sequence* number in the header is `u32` (~4 G rotations) | E21, G12b |
| ✚ **`clock_epoch_lo`** | `u8` in the header | Low 8 bits of `boyko_diag::clock_epoch()`. Reconstructed by the sink against the live epoch; at most one boundary can lie between producer and sink (`substrate/clock-source`) | S4 |
| `tsc` | `u64` | ~195 years | E8 |

*(`BinarySink` itself — Decision 22, its widths' owner document and its G12c revert clause — is `logging/sink-lifecycle`'s. The **audit rows** live here, because deferring the widths to the format document is exactly what let v3 claim "every integer was audited" while auditing none of them.)*

---

## Data structures — the producer half

*There is no `control.rs`: `CONTROL` is declared in `target.rs`, beside the `TargetId` whose invariant makes its `get_unchecked` sound.*

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
/// the PAD sentinel (§Algorithms A6).
#[repr(C, packed)]
struct RecordHeader {
    site:  *const LogSite, // 8
    tsc:   u64,            // 8  raw boyko_diag::clock::ticks(); scaled by the sink
    len:   u16,            // 2  TOTAL record bytes incl. header — the walk step
    flags: u8,             // 1  STR_TRUNCATED | SUPPRESSED_FOLLOWS | TOO_LARGE
    /// Low 8 bits of `boyko_diag::clock_epoch()` (S4). SPENDS v3's `_pad`, so
    /// the header does NOT grow and the 20 B assert below is unchanged. Read as
    /// a register-resident global, not probed, so the <= 15 ns row is untouched.
    /// 8 bits suffice: the sink is at most one park interval behind the
    /// producer, so at most one epoch boundary can lie between them and the
    /// full u32 is reconstructed against the live epoch. The sink RENDERS it
    /// beside every timestamp, so a record straddling a suspend is legible
    /// rather than merely wrong.
    clock_epoch_lo: u8,    // 1
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
///
/// RENAMED from `Lane` to `LogLane` (S11): the lane *identity* is now
/// `boyko_diag`'s and one name may not mean two things. There is no `owner`
/// field any more — the index comes from `boyko_diag::lane()`, which is
/// unique per live thread by construction (S3).
#[repr(C, align(64))]
pub(crate) struct LogLane {
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

    // ── line 2: LANE-OWNED STATISTICS (producer writes, consumer folds) ──────
    /// `boyko_diag::LossCell` — written BY THE LANE OWNER with NO lock prefix
    /// and no RMW (S8). v3 used saturating `AtomicU32` on the argument that an
    /// 8-byte RMW costs more; on x86-64 `lock xadd` costs the same at 4 and 8
    /// bytes, and a single-writer cell needs no RMW at all — so the rejection
    /// does not survive, and with `u64` the `SATURATED` token (which a reader
    /// could never COMPARE) stops existing. The cells are `AtomicU64` accessed
    /// `Relaxed` by the owner rather than plain `u64`: substrate F5 — a plain
    /// cell read across threads is UB regardless of what x86-64 does, and Miri
    /// reports it, while `Relaxed` lowers to the identical `mov` pair.
    /// The consumer folds into a `LossTotal` with `fetch_sub(observed)`, never
    /// `store(0)`: a `store` loses any increment landing between load and clear.
    /// (Producer-side lost updates are substrate Q2, still OPEN.)
    loss:          [LossCell; LOSS_CLASSES],   // Overflow | Unclaimed | Refused | Sink
    /// Deliberately NOT emitted (Decision 20). NOT a loss, so NOT a LossClass —
    /// counted separately so conflating the two cannot make either number a
    /// liar. The property `emitted == drained + dropped + sampled_out` depends
    /// on the separation being exact.
    sampled_out:   AtomicU64,
    _pad2:         [u8; PAD2],

    // ── payload ──────────────────────────────────────────────────────────────
    buf: UnsafeCell<[MaybeUninit<u8>; LANE_BYTES]>,
}
const _: () = assert!(core::mem::align_of::<LogLane>() == 64);
const _: () = assert!(core::mem::offset_of!(LogLane, read) == 64);
const _: () = assert!(core::mem::offset_of!(LogLane, loss) == 128,
    "statistics are a third partition: producer writes, consumer folds");

// SAFETY (manual Sync for LogLane):
//   1. WRITE side: exactly one thread ever writes `buf` or `write` — the one
//      whose `boyko_diag::lane()` returns this index. The uniqueness is the
//      SUBSTRATE's, and it is single-writer by construction on both of its
//      two paths (S3): a pool worker's index IS its dense `worker_id`
//      (`boyko_threadpool::worker::worker_main`, one live thread per id by the
//      pool's own construction), and every other thread's index comes from
//      `claim_lane()`'s load-then-CAS over the spare slots, whose loser
//      retries a different slot. No two LIVE threads hold one index, so two
//      PRODUCER THREADS on one lane is unrepresentable.
//      NOTE what changed from v3: the exclusivity used to be argued from a
//      CAS on a field of THIS struct. It is now argued from a CAS in
//      `boyko_diag`. That is not a weakening — it is the same argument with
//      ONE owner instead of two, which is the point: two registries would let
//      the profiler hand a reclaimed index to a new thread while this crate
//      still believed the old one owned it.
//   1b. Re-entrant emit on ONE thread is separately excluded: no user code can
//      run between lane acquisition and the `Release` store, because `dsp!`
//      runs in argument position and encoding operates on POD and `&str` only
//      (Decision 13). `debug_assert`ed by IN_EMIT.
//   1c. `LogPod` does not weaken 1b: the ENCODE half is GENERATED FROM
//      `LogValue` field by field (B10 — the blanket `copy_nonoverlapping` of
//      `size_of::<Self>()` is deleted, because it copied padding and the sink
//      then materialised a `&[u8]` over uninitialised memory), and the
//      user-supplied `fmt_pod` runs on the SINK thread from the staging arena,
//      in the same position as `site.decode` (Decision 19b, test 24).
//   1d. `SAMPLE_CTR[lane]` is written only by the lane's owner (the ROW INDEX
//      IS THE LANE INDEX), with Relaxed load/store and never an RMW, so it
//      inherits clause 1 verbatim (Decision 20).
//   1e. `read_cached` is a `Cell<u32>` READ AND WRITTEN ONLY BY THE PRODUCER
//      that owns the lane — it is the reason `LogLane` is not `Sync` by
//      derivation, and clause 1's exclusive-write argument covers it exactly:
//      a second producer thread cannot exist, and the consumer never names it.
//      Its value is a STALE lower bound on `read`; staleness is safe because
//      it can only make the producer refuse space it actually had, never grant
//      space it did not (Decision 5's induction) (F23).
//   1f. `write_cached` is the mirror clause: `Cell<u32>`, read and written
//      only by the thread currently holding the CONSUMER role (clause 2), and
//      a stale value can only make the consumer drain less than was available,
//      never read past `write` (F23).
//   2. READ side: exactly one thread reads `buf` and writes `read` — the
//      holder of `DRAIN_OWNER`, a single CAS'd token (Decision 24, B5).
//      All FOUR consumers claim it identically: the sink thread, `drain()`
//      under `Manual`, `log_drain_system` under `Scheduled`, and the crash
//      drainer. v3 instead CAS'd `SINK_STATE` out of {Exited, NotBooted,
//      Manual} and called all three quiescent; `Manual` is NOT — it means an
//      arbitrary thread may be inside `drain()` right now, so a panic
//      elsewhere started a SECOND consumer over these very bytes. CASing the
//      role removes the gap between "a state that correlates with exclusivity"
//      and "exclusivity".
//   3. Payload visibility: bytes written before `write.store(_, Release)` are
//      visible to a thread observing that value via `Acquire`. The consumer
//      never reads past its observed `w`, AND never advances `read` over bytes
//      it has not yet copied out (Algorithms C).
//   4. Retire: `boyko_diag::release_lane()` marks the SUBSTRATE's slot
//      RETIRING, on the producer thread, after its last write. The consumer
//      calls `boyko_diag::reclaim(i)` only after observing RETIRING and
//      `read == write` for THIS lane, so no producer write can follow a
//      reclaim. The state lives in one place, so the profiler cannot reissue
//      the index while this crate still holds undrained bytes.
unsafe impl Sync for LogLane {}

/// NOT a constant of this crate any more (S3): 80 in `dev`/`editor`, 32 in
/// `shipping`/`shipping-min`, from `BOYKO_PROFILE` via `boyko_diag/build.rs`.
/// v3's `MAX_LANES = 128` is deleted, along with its `option_env!` override.
/// (The 32 is substrate BLOCKER Q1 — unsound against a worker-anchored
/// topology whose floor is 66. This crate consumes the resolved value.)
pub(crate) use boyko_diag::LANE_COUNT;
pub(crate) const LANE_BYTES: usize = 16 * 1024;  // power of two: MASK arithmetic
const MASK:          u32   = (LANE_BYTES - 1) as u32;
/// Usable span. ONE slot reserved so `used == CAPACITY` cannot be confused with
/// `used == 0` without a third variable — and, critically, so that
/// `avail = CAPACITY - used` in Algorithms A6 cannot underflow (F6).
const CAPACITY:      u32   = (LANE_BYTES - 1) as u32;
const ERROR_RESERVE: u32   = (LANE_BYTES / 8) as u32;   // 2 KiB, Error-only tail
const _: () = assert!(LANE_BYTES.is_power_of_two());
const _: () = assert!(ERROR_RESERVE < CAPACITY);        // else no non-Error record ever fits

pub const LANE_ARRAY_LEN: usize =
    if (GLOBAL_CEILING as u8) == 0 { 0 } else { LANE_COUNT as usize };
static LOG_LANES: [LogLane; LANE_ARRAY_LEN] = [LogLane::NEW; LANE_ARRAY_LEN];

// ────────────────────── boyko_log/src/sink/ecs.rs (B2) ────────────────────────
// `HandoffRing` / `ECS_HANDOFF` — the sink -> ECS transport, specified in full
// by `logging/sink-lifecycle` (Decision 26: layout, capacity, ordering,
// overflow, budget row, SAFETY). Same shape and same wrap rule as `LogLane`:
// no new protocol, one new instance. Single producer = the DRAIN_OWNER holder;
// single consumer = `log_drain_system` holding `ResMut<LogRing>`.

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
/// `.bss`-zero == level `Off`, shift 0, sync off == disabled until boot arms it
/// — and, after S13, until the LAUNCH FLAG enables it. This array IS logging's
/// runtime flag word; `ARM_MASK` is the profiler's twin.
static CONTROL: [AtomicU8; MAX_TARGETS] = [const { AtomicU8::new(0) }; MAX_TARGETS];
/// Monotone; `Release`-added on every control change. A UI POLLS this to know
/// it must repaint — the O(1) stand-in for the change detection the refused ECS
/// route would have given (Decision 23). RENAMED from `CONTROL_EPOCH` (S11):
/// three unrelated things were called "epoch" across the two plans, and this
/// one is the control-change counter, not a clock epoch and not a flush
/// sequence.
static CONTROL_EPOCH_CTR: AtomicU32 = AtomicU32::new(0);   // read via control_epoch()

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
/// Row count follows `boyko_diag::LANE_COUNT`, which is 80 in EVERY profile
/// (Q1): 40 KiB everywhere (v3: 64 KiB at 128 lanes).
static SAMPLE_CTR: [[Cell<u16>; MAX_TARGETS]; LANE_COUNT as usize];
```

> **`LOSS_CLASSES` is the lane-side SUBSET, not the enum.** The comment above enumerates four (`Overflow | Unclaimed | Refused | Sink`) because those are the classes a *producer* can raise; `Rotation` (E21) is sink-side. `substrate/loss-vocabulary` owns the full **8-variant** `LossClass`, so `LOSS_CLASSES` and the lane's array extent must be reconciled against it at D0 — a lane array sized by a subset while the enum grows is a silent index error, and this note exists so the reconciliation is a line item rather than a discovery.

---

## Public API — the emission slice

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

/// Renders a `Display` into a stack buffer owned by the ARGUMENT EXPRESSION.
/// Expansion form pinned in Decision 13 — the naive block form does not
/// borrow-check (F22).
#[macro_export] macro_rules! dsp { ($e:expr) => {...}; ($e:expr, $n:literal) => {...} }

// `report!` IS DELETED (S1). The measurement channel belongs to the profiler
// end to end; its durable output is an artifact, never stdout. NOTHING in this
// crate writes stdout. `OUT_LOCK` survives — its callers are enumerated by
// `logging/sink-lifecycle`.

// ── values ────────────────────────────────────────────────────────────────────
pub trait LogValue: private::Sealed {
    const MAX_ENCODED_LEN: usize;
    fn encoded_len(&self) -> usize;
    unsafe fn encode(&self, dst: *mut u8) -> usize;
}
// impls: {i,u}{8,16,32,64,128}, f32, f64, bool, char, &'static str, &str.

/// Game-extensible POD values. Blanket-bridged into `LogValue`, so sealing is
/// preserved and Decision 13's structural property is untouched: the encoder
/// is generated from `LogValue`, `fmt_pod` runs on the sink (D19b, test 24).
/// `POD_LEN` is the SUM OF FIELD ENCODED LENGTHS, not `size_of::<Self>()`, so
/// no padding byte ever reaches a sink (B10).
pub unsafe trait LogPod: Copy + Send + Sync + 'static {
    const POD_LEN: usize;
    /// # Safety
    /// `dst` valid for `POD_LEN` writes; the impl writes exactly that many
    /// INITIALISED bytes.
    unsafe fn encode_pod(&self, dst: *mut u8);
    fn fmt_pod(bytes: &[u8], f: &mut LogFormatter);
}
// boyko_macros: #[derive(LogPod)] — requires #[repr(C)] and all-LogValue
// fields, REJECTS dynamic-length fields (`&str`), and generates `encode_pod`
// field-by-field plus `const _: () = assert!(POD_LEN == Σ field lengths)`.

// ── runtime control (any thread, no lock, no restart — Decision 23) ───────────
pub fn target_control(id: TargetId) -> TargetControl;
pub fn set_target_control(id: TargetId, ctl: TargetControl);   // CAS, preserves siblings
pub fn set_target_level(id: TargetId, lvl: Level);             // CAS, preserves shift+sync
pub fn control_epoch() -> u32;                                 // O(1) repaint signal
```

*The `dyn_*` macros, `register_dynamic_target` / `find_target` / `targets()`, the `LogPod` derive's game-facing framing and `apply_control_spec` are `logging/game-facing-surface`'s and `logging/sink-lifecycle`'s; they appear above only where a producer-path property depends on them.*

---

## Algorithm A: `emit` — the producer hot path

```
1. GATE (inlined into the caller)
   a. T::STATIC_CEILING       >= LVL           — const, folded  [ABSENT for dyn_* — D18]
   b. $crate::GLOBAL_CEILING  >= LVL           — const, folded
   c. ctl = CONTROL[T::ID].load(Relaxed);  (ctl & 0x07) >= LVL  — 1 B L1 load + and + cmp
   Fail ⇒ nothing. Arguments NEVER evaluated (&& short-circuit).
   [Arguments, incl. any `dsp!`, are evaluated HERE, before step 2.]
   // S13: (a) and (b) are the COMPILE CEILING and delete the site AND its
   // operands. (c) is the RUNTIME FLAG and is the floor: it cannot be driven
   // to zero by turning the flag off, because a flag has to be read.

1b. SYNC ROUTE (bit 7 of ctl, predicted not-taken)
   ctl & 0x80 ⇒ format on this thread, write_oracle_line() (BOUNDED — D9c),
   TARGET_STATS.sync_routed += 1, return.  ~200+ ns, per-target opt-in (D20).

2. RATE (Warn/Error only) — code-indexed and site-indexed, NOT lane-indexed,
   so it stays ahead of step 3 and a suppressed record never pays a lane claim
   Once/OnceCounted && FIRED.load(Relaxed) ⇒ [OnceCounted: suppressed RMW] return
                                             // per-SITE static, private line (F11)
   EveryN(n)  ⇒ (count.fetch_add(1) & (n-1)) != 0 ⇒ return   // n is pow2 (X3)
   MinInterval⇒ policy RMW
   Every      ⇒ skip
   Downstream code: idx = idx_cell.load(Relaxed); UNASSIGNED ⇒ #[cold] mint (D19)
                    idx == CODE_IDX_EXHAUSTED ⇒ Every semantics, no rate state,
                    codes_unindexed += 1 (M3)

3. LANE  = `boyko_diag::lane()` (S3 — a Cell<u16> TLS read, no Drop, no claim
   scan in this crate). UNCLAIMED ⇒ #[cold] `boyko_diag::claim_lane()`.
   Still none ⇒ Warn/Error: write_oracle_line() (synchronous fallback with the
   durable fan-out, M26 + B9); else LossClass::Unclaimed += 1; return.
   // MOVED AHEAD OF SAMPLING (B4). v3 resolved the lane at step 4 while step 3
   // already indexed `SAMPLE_CTR[lane][target]` — the row index was read
   // before it existed, and an unlaned thread (E6) had no row at all. Only
   // SAMPLE is lane-indexed, so this is the minimal reordering that fixes it:
   // RATE keeps its position, and a rate-suppressed record on a fresh thread
   // still costs no lane claim.

4. SAMPLE (k = (ctl >> 3) & 0x0F; predicted not-taken when k == 0)
   c = SAMPLE_CTR[lane][target].get().wrapping_add(1)
   SAMPLE_CTR[lane][target].set(c)                    // plain load/store, NO RMW
   (c & ((1<<k)-1)) != 0 ⇒ sampled_out += 1; return
   // NOTE, and it is a user-visible property rather than an implementation
   // detail: ARGUMENTS WERE ALREADY EVALUATED AT STEP 1. Sampling suppresses
   // DELIVERY, never argument evaluation. A side-effecting argument runs on
   // every occurrence regardless of `k`. G10e asserts both numbers in one leg
   // — 1000 evaluations, 500 delivered — which is the distinction v3's G4 leg
   // (d) collapsed by asserting 500 for a quantity that is 1000 (B4).

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
           // S8 form. The lane owner is the SINGLE WRITER of its own LossCell:
           //   cell = &loss[LossClass::Overflow]
           //   cell.count.store(cell.count.load(Relaxed) + 1,          Relaxed)
           //   cell.bytes.store(cell.bytes.load(Relaxed) + need as u64, Relaxed)
           // u64, NO lock prefix, NO saturation check, NO ceiling state.
           // (See the divergence note below: the v4 listing still carried v3's
           //  `dropped.load(Relaxed) != u32::MAX` + `fetch_add` lines here,
           //  which Decision 5, Decision 21 and S8 all delete.)
           return
       }
   }
   if pad >= HEADER_BYTES { write PAD header (site = null, len = pad) at off }
   w = w.wrapping_add(pad); off = w & MASK   // now off == 0 or tail >= need

7. WRITE   write_unaligned(off, RecordHeader{ site, tsc: boyko_diag::ticks(),
                                              len: need, flags,
                                              clock_epoch_lo: epoch as u8 })
           args.encode(off + HEADER_BYTES)
8. PUBLISH write.store(w.wrapping_add(need), Release)
```

- **Complexity** O(1); O(len) memcpy for inline `&str`.
- **Cache — the isolated figure and the joint one, together, because quoting one without the other is how a budget gets believed.** Strictly sequential streaming writes into the ring tail. **In isolation** the working set is the `CONTROL` line, the producer line, the lane's `SAMPLE_CTR` row segment (one line) and 1-2 ring-tail lines — **≤ 4 lines**, unchanged from v2, because the sampling row is the only addition and it is one line and producer-private. A `Once` site's `FIRED`/`OnceSite` static is a fifth line only on the `Warn`/`Error` path, which is not the budgeted path. **Jointly with the profiler armed the same producer also touches `ARM_MASK`, the `ZoneLane` control line and the sample tail, for 7-8 distinct lines** (`seam/joint-cost`) — and the shared TLS slot means 1, not 2, of those lines is the lane id. `LOG_LANES`, `CONTROL` and `SAMPLE_CTR` have compile-time-known addresses — no pointer chase.
- **Branching** 3 (or 2, dynamic) predicted-not-taken gates + sync + rate + sample + wrap + space. `budget` is a `saturating_sub`, i.e. `sub` + `cmov` — still branchless. The sync and sample branches are not-taken in every default configuration, so I-cache pressure is one extra `cmp/jcc` pair each.
- **Inlining** steps 1-3 `#[inline]` (must fold). Steps 4-8 in `#[inline(never)] fn emit_impl<A: LogArgs>` — monomorphised per argument-tuple type. Blanket `#[inline(always)]` would replicate ~60 instructions at every site and bloat L1i, which principle 7 forbids on measurement grounds.
- **SIMD** none wanted: the payload is ≤ 2 KiB and moves by `copy_nonoverlapping`, which already lowers to the best available move sequence. There is no vectorisable reduction anywhere in `boyko_log`.

---

## Algorithm B: Lane resolution / retire — the claim scan lives in `boyko_diag` now *(S3)*

```
RESOLVE (hot, one TLS read):
  id = boyko_diag::lane()                       // Cell<u16>, NO Drop, no allocation
  if id == LANE_UNCLAIMED { id = boyko_diag::claim_lane()? }   // #[cold], once/thread
  lane = LOG_LANES.get(id as usize)?            // None in the `off` build (len 0, F21)
  first touch of this lane by this thread ⇒
      seed SAMPLE_CTR[id][t] = (id * 0x9E37) as u16 for all t  // phase break (D20)

RETIRE (producer thread, explicitly, after its last write):
  boyko_diag::release_lane()                    // marks the SUBSTRATE slot RETIRING
  // NOT a `Drop` guard. v3 installed a `thread_local!` with a destructor; that
  // was the mechanism the profiling plan refused AND the sole source of this
  // plan's "<= 1 allocation on first emit" row. Deleting it takes that row to 0
  // (S3). A thread that simply exits without calling `release_lane()` leaks its
  // spare for the process — bounded at 14 x LANE_BYTES, counted as
  // `lanes_leaked`, printed in the census. That is the price of not having a
  // destructor, and it is paid in a bounded, counted, printed quantity rather
  // than in an allocation on every thread's first emit.

RECLAIM (consumer, per drain, after staging):
  if boyko_diag::lane_state(i) == RETIRING && read == write {
      boyko_diag::reclaim(i)                    // ONE registry owns the transition
  }
```

**What this crate no longer contains**: `MAX_LANES`, the `hash(thread_id)` spread, the `load`-then-CAS scan over `LANES`, the `owner` field, `MY_LANE`, and the TLS guard type. Five deletions, one dependency.

**S13 note on the "first touch" seeding.** Seeding `SAMPLE_CTR[id][*]` is a **write**, so it commits pages. It happens on the first *emit* through a claimed lane, which cannot occur while the flag is off (gate (c) of Decision 2 fails first). Pre-touching lane buffers on the enable path is admissible and is a rung decision; touching them while the flag is off is **not**.

---

## Multithreading model

*Carried whole. The table is one object with one ordering rationale, and splitting it across the producer and consumer files is how two files come to disagree about a memory ordering; `logging/sink-lifecycle` cites this table rather than restating any row of it.*

| Datum | Sharing | Ordering | Why |
|---|---|---|---|
| `LogLane::buf` | SPSC | none (guarded by `write`) | payload published by the cursor's Release |
| `LogLane::write` | P→C | `Release` / `Acquire` | the happens-before edge for the payload |
| `LogLane::read` | C→P | `Release` (after staging) / `Acquire` | frees space only once bytes are copied out (B1) |
| `read_cached` / `write_cached` | private `Cell`, single-role | none | the half that actually buys throughput; SAFETY clauses 1e/1f cover them (F23) |
| **lane identity** | owned by `boyko_diag` | `Cell<u16>` TLS read (no atomic); `claim_lane` is load-then-CAS `Acquire`, `release_lane` `Release` | **not this crate's datum any more** (S3). Contended once per thread lifetime, on a `#[cold]` path, over 14 spares |
| `LossCell[class]` (per lane) | **single writer** = lane owner; consumer folds | owner-side `AtomicU64` `Relaxed` load/store (no lock prefix, no RMW — substrate F5); `fetch_sub(observed)` on the `AtomicU64` total | own cache line (S8). `fetch_sub` never loses a *consumer*-side concurrent add — a `store(0)` would. The **producer**-side lost-update window is substrate **Q2**, still OPEN |
| `sampled_out` | P adds, C folds | `Relaxed` | not a `LossClass`; kept separate so `emitted == drained + dropped + sampled_out` stays exact |
| **`DRAIN_OWNER`** | MPMC, once per drain pass | CAS `AcqRel` / `Acquire` on failure; RAII `Release` | **the object clause 2 of the `LogLane` SAFETY block is about** — so it is the object that is CAS'd (B5). Uncontended in every normal configuration; contended only when a panic races a drain |
| `CONTROL[i]` | MP-read, rare CAS write | `Relaxed` read, `AcqRel` CAS | a stale ceiling for one record is documented as acceptable; the CAS preserves sibling bit-fields (D14) |
| `CONTROL_EPOCH_CTR` | 1W-ish / MR | `Release` add / `Acquire` load | derived and monotone; carries no state, so it cannot diverge from `CONTROL` |
| **`ECS_HANDOFF.write` / `.read`** | SPSC: producer = `DRAIN_OWNER` holder, consumer = `log_drain_system` | `Release` / `Acquire`, both cursors | identical to `LogLane`'s pair — one protocol, two instances (D26, B2). Overflow is a counted refusal (`LossClass::Sink`, `W0117`), never a silent drop |
| **`ONCE_SITES` head / `next`** | MP push, insert-only | CAS `AcqRel` push, `Acquire` walk | one push per site per **process**, on the `#[cold]` single-fire branch; the census walks it (M1). Nothing is ever removed or freed. **Why the push is `AcqRel` and not `Relaxed`** — the same shape as the `DYN_NAMES[i].hash` row below: a pusher publishes `site`/`suppressed` before the CAS that links `next`, so a walker observing a non-null pointer via `Acquire` observes a **complete node**. Every node is a `'static`, so nothing is ever freed under the walker, and the census tolerates a node appearing between two walks. The full SAFETY block is at the struct, in Decision 8 point 2. `OnceSite` carries **no `fired` field**: the latch is the sibling `static FIRED: AtomicBool` of Decision 8, and restoring the monolith's duplicate would give one site two latch bits |
| **`PRE_FLUSH[n]`** | MP claim, MR call | CAS `AcqRel` claim, `Acquire` load | eight slots, claimed at **enable** (S5 + S13's move off boot); a ninth is `E0118` |
| `SAMPLE_CTR[lane][t]` | **single writer** = lane owner | `Relaxed` load + store, **never an RMW** | the row index IS the lane index ⇒ SAFETY clause 1 applies verbatim; no sharing, no lock prefix |
| per-site `FIRED` (`Once`) | MP | `Relaxed` load; one lifetime `swap` | steady state is a pure load from a **site-private** line (M11, F11) |
| `RATE[idx]` (`EveryN`/`MinInterval` only) | MP | `Relaxed` RMW | opt-in; cost documented at the declaration site |
| `CodeIdx::Dynamic` cell | MP | CAS `UNASSIGNED→RESERVED`, `fetch_add`, `store(Release)`; readers `Acquire` | reserve-then-publish: dense, no leaked indices, one CAS per code per process (D19) |
| `DYN_NAMES[i].hash` | MP insert-only | bytes+len stored, then `hash.store(Release)`; readers `Acquire` | a reader seeing a hash sees a complete name |
| `TARGET_STATS[i]` | C writes, MR reads | `Relaxed` | one writer (the consumer-role holder); readers tolerate one-drain staleness, which the census states |
| `SINKS[n].state` / `.filter` / `.floor` | MW stores, C reads | `Relaxed` | policy only; acting on a one-drain-stale filter is a documented property (G13), not a race |
| `SINK_REQ` | MP producers, 1 consumer | `AcqRel` seq; writes under `OUT_LOCK` | cold, human-initiated, bounded at 16; full ⇒ `E0107` |
| `FLUSH_SEQ` / `FLUSH_ACK` / `SINK_STATE` / `SINK_EXITED` | 2-way | `AcqRel` / `Acquire` | completion must be observed, not guessed. **`SINK_STATE` no longer carries the exclusivity proof** — `DRAIN_OWNER` does (D24, B5) — so it has one job, lifecycle, and `CrashDraining` is deleted from it |
| `OUT_OWNER` (`OUT_LOCK`) | MP | CAS `Acquire` / `Release`; **bounded acquire, RAII release, re-entrancy detected** | callers: `write_oracle_line` (console sink, sync-routed targets, exhaustion fallback, pre-boot/post-shutdown severe records, panic message, flush timeout) and `SINK_REQ` writes. `report!` is gone (S1); the lock is not. The Vulkan messenger is **not** a caller (D9b) |
| Sink array kinds, `LogConfig` | boot-published | one `Release` at boot | kinds never mutate after boot; only state/filter/floor do |

**Data-race freedom.** No lane has two producer *threads*: `boyko_diag::lane()` returns an index unique per live thread, dense-by-construction for pool workers and CAS-claimed for everyone else (S3). No lane has two producers *re-entrantly* on one thread (no user code runs inside the open window — Decisions 13 and 19b, `debug_assert`ed and asserted by test 24; and after B10 the `LogPod` encoder is generated from `LogValue`, so it is ours). **No lane ever has two consumers**: all four consumers — sink thread, `drain()`, `log_drain_system` under `Scheduled`, and the crash drainer — acquire the **same CAS'd `DRAIN_OWNER` token** (Decision 24). v3 inferred exclusivity from `SINK_STATE ∈ {Exited, NotBooted, Manual}` and was wrong about `Manual`, which is a state in which a consumer may be *running*. `SAMPLE_CTR` rows and `LossCell`s inherit the single-writer property from the lane index. Payload visibility rests on the `Release`/`Acquire` cursor pair, and the consumer never reads past its observed `w` **nor advances `read` over bytes it has not staged**. Reclaim is ordered by `RETIRING` (in `boyko_diag`) being stored after the producer's last write and observed only after the consumer has drained to `write`. `ECS_HANDOFF` repeats the same argument one level up, with the producer side pinned to the `DRAIN_OWNER` holder and the consumer side to the scheduler's `ResMut` exclusivity (Decision 26).

**`Send`/`Sync`.** `LogLane: Sync` and `HandoffRing: Sync` via documented manual impls; `OnceSite`, `DynSlot`, `TargetStatCell`, `SinkSlot`: `Sync` via impls whose SAFETY blocks name the single-writer, insert-only or atomic-only argument. `TargetId`, `TargetControl`, `WarnCode`, `ErrorCode`, `PanicCode`, `Level`, `boyko_diag::SessionId`: `Copy + Send + Sync`. **`LogRing` and `LogCensus` are `Send + Sync` by a MANUAL `unsafe impl` with a SEND10-shaped argument, not by derivation** — they hold `VmColumn`, which `crates/boyko_ecs/src/ecs/memory/vm_column.rs:70` states is **NOT `Send`/`Sync`**, while `resource.rs:42` requires both of every `Resource` (B1). *(Both re-verified this session, verbatim.)* The argument, its named holder set, and its `const _` `assert_send_sync` pin are `logging/game-facing-surface`'s; the load-bearing clause is that **the sink thread never touches either type** — it writes `ECS_HANDOFF`. `LogStats` is `Copy` POD and derives both. **No `!Send` handle exists** (Decision 12). `LogPod: Send + Sync` is required so a game type cannot smuggle a thread-affine value onto the sink thread.

---

## Divergences found at the carve

Three places where the source contradicted itself. None is silently "fixed" and none is silently propagated, because the first is how a contradiction survives a split and the second is how a repair introduces a new lie.

1. **Algorithm A step 6's drop accounting was still v3's.** The v4 listing carried `if dropped.load(Relaxed) != u32::MAX { dropped.fetch_add(1, Relaxed); dropped_bytes saturating-add need; }` — a saturating `AtomicU32` RMW. **Decision 5 (S8), Decision 21's `dropped` row and the multithreading table all say the opposite**: `u64`, single-writer, no lock prefix, no saturation, and the `SATURATED` census token struck. Three statements to one, and the odd one out is the algorithm listing, which v4's own Decision 5 rewrote around. Carried in the S8 form, with the superseded lines shown in the comment so a reader diffing against the monolith sees why the text changed.

2. **`LOSS_CLASSES` names four classes; `LossClass` has eight.** The lane's `loss` array is sized by a lane-side subset (`Overflow | Unclaimed | Refused | Sink`) while `substrate/loss-vocabulary` owns an 8-variant enum, and Decision 5's own prose adds a fifth (`Rotation`, sink-side). Not a contradiction yet — a subset is legitimate — but it becomes a silent index error the first time someone indexes the lane array by a `LossClass` discriminant. Flagged as a D0 reconciliation line item rather than resolved here, because the enum is not this file's to define.

3. **`OnceSite` carried TWO latch bits for one site.** The monolith states the `Once` latch as a standalone sibling static in four places — `docs/LOGGING-SYSTEM-PLAN.md:419` ("a macro-generated `static FIRED: AtomicBool` **beside** the call site's `LogSite`"), `:426` ("Cost: 1 byte of `.bss` per `Once` site"), `:429-430` (the two-line pseudocode) and `:1008-1010` ("The per-SITE `Once` latch is NOT a field here … The macro expands a sibling `static FIRED: AtomicBool` beside each `Once` site — per-site, private line, **1 byte**") — and then **also** puts `fired: AtomicBool` inside `OnceSite` at `:1312`. Those are two latch bits for one site and both cannot be the single latch; the 1-byte cost claim is only true of the standalone form. **This file keeps Decision 8's version and drops the duplicate field**, which is why the struct above has three fields and not four. The consequence is written into the code, not left to inference: the SAFETY block's publication clause names `site`/`suppressed`, and the `ONCE_SITES` row of the multithreading table says in terms that the node carries no `fired` — so an implementer reading either one cannot restore the duplicate by accident. **The six-line SAFETY block itself was lost at the carve and is restored above**, verbatim except for that one word; it is the publication-before-CAS argument, and without it the table's `AcqRel` is a bare assertion.

**One citation repair** carried from `logging/budgets-and-invariants`' repair pass, because it is cited by Decision 8 above: `host.rs`'s "a RELEASE-build degrade-to-Off must be observable" is at **`:230-234`**, not `:228-233`. The argument is unchanged; only its address was wrong.
