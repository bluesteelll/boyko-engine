# Schedule determinism (lane K) - review of rev 2

VERDICT: REVISE; BLOCKING=0; IMPORTANT=3

# Architecture review: kernel lane K, rev 2

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED

Rev 2 closes C1 and most of W1–W5 and O1–O5. No blockers remain. There are three important remarks: W-A affects K1b, and W-B and W-C affect K3b/K3c. K1a, K2a–K2c and K3a can start as specified.

## Status of the rev 1 remarks and rulings (checked in `D:/wt/joltab`)

- **C1 ✅**
  - `IsEnabled` has a no-op `init_access` today (`iters/query/data_is_enabled.rs:170-176`), and D5 fixes that.
  - The only runtime-term APIs are `Query::with_enabled`/`without_enabled` (`query.rs:200,215`) and the `QueryView` mirrors (`query_view.rs:283,298`).
  - A `QueryView` can only be reached through `EcsMaster::query(&mut self)` / `query_filtered` (`ecs_master.rs:825,929`).
  - The census list matches a grep of the tree, and there are 0 production sites.
  - The exposure arithmetic is consistent: hwrt 10+9+9+8+5 = 41, software 9+8+8+7+5 = 37.
  - The only scheduled `Commands` holders on the host are the four the plan names (`asset_refcount.rs:95,405`, `visibility_sync.rs:144`, `snap_interpolation.rs:125`). The other `Commands` uses are one-shot closures in `csm_plugin.rs`/`shadow_plugin.rs`.
  - `EnginePlugins` adds no `boyko_ui` system, so the UI enable tags are correctly left out.
- **W1(a) ✅** `HAS_DEFERRED` is a required const with no default (`system/system_param.rs:112`). An omission therefore cannot defeat the per-parameter accounting. The tuple forwarder is at `params/tuple_impl.rs:110-123`.
- **W1(b) ✅** No production code registers a `GpuCompute` system: `.gpu()` appears only in `boyko_render/tests/*`.
- **W2 ✅** I re-derived the Kahn order Z N Q S D, the waves {Z,Q} → {N,D} → {S} and the apply sequence [D,S]. Under a split retire the case can produce [S,D].
- **W3 ✅ mechanism.** It replaces the ruling's "unattached threads route to lane 0" with `&mut`, which is justified by self-found (b): `EcsMaster: Sync` (`ecs_master.rs:1287-1288`) and the unattached arm (`tls.rs:410-411`). This conflicts with an earlier ruling; see W-A.
- **W4 ◐** The CI step and the CLAUDE.md leg are named, but the test layout they run cannot be built as described; see W-B.
- **W5 ◐** FROZEN plus the digest is sound, but the RESOLVED and ALIASES lists are unguarded; see W-C.
- **O1, O2, O3, O5 ✅. O4 ◐** (see O-1).
- **Critic's questions 1–4 ✅**
  - Q1: the cull writes only `stats.*` and, in Auto mode, `clusters_enabled` (`light_policy.rs:211-220`). The gates write disjoint fields (`light.rs:1026-1030,1156-1159`, `csm_caster.rs:573-575`, `shadow_atlas.rs:1176-1178`, `ssao_config.rs:283-285`).
  - Q4: the engine default is `EventDispatcher::new(1)` (`ecs_master.rs:429,476`).

## Remarks

### 🔴 Critical
None.

### 🟡 Important

#### W-A. D2c's `send_event(&mut self)` overturns two recorded decisions without citing either
**Where:** D2c; the "Changes" table row for W3; self-found (b).

**Problem:**
- **Aether ruling E5 (ballot AB-3, 2026-08-30).** It already measured this exact race (6/6 release runs lost 2.3–28.7 % of events, every send returned `Ok`). It chose a claimed host lane with one claimer enforced. It explicitly rejected refusing the unattached sender, because that "breaks the escape hatch the engine itself documents … the host thread has a legitimate need to send" (`docs/aether-v2/DECISIONS.md:539-580`; `EVENTS.md:125-139`).
- **U-21 / D-E20 in the unified plan.** They keep `&self` with a worker → `Err` contract, and the named gate is `tests/event_send_from_worker.rs` asserting `Err(EventSendOffDispatcher)` (`docs/unification/…02-ORDER-OF-WORK.md:323`; `01-KERNEL-CONTRACT.md:613`; `00-OVERVIEW.md:128`).
- **Rev 2's position.** It makes every non-writer send require `&mut` and removes the unattached route. Its Context cites neither decision.

**Consequence:**
- The escape hatch documented at `event_writer.rs:98-103,114-115` and `commands.rs:351,364` ("main-thread / FFI callers use `EcsMaster::events().send_event`") stops compiling for any holder of `&EcsMaster`.
- Aether R4's E5 host-lane contract (and E4's `lanes N` floor, which writer lanes make meaningless) can no longer be built.
- D-E20's gate text in 02 describes an API that no longer exists.
- Whoever next implements R4 or reads 02:323 builds against the wrong surface.

**Confidence:** CONFIRMED (the citations above; `send_event` is at `event_dispatcher.rs:285-294` today).

**What is needed:**
- State the fork: `&mut` (rev 2) versus E5's CAS-claimed host lane at a fixed index kept alongside writer lanes.
- Price what `&mut` removes. Note that host-thread sends are timing-dependent against frame boundaries and so already sit outside K1, which argues for `&mut`.
- Route the overturn of E5 and the U-21 amendment to their deciders.
- Add the documents that must change to K1b's file list: `02:323`, `01:613`, `aether-v2/{DECISIONS,EVENTS}.md`, and `constants.rs:396-432`. The last one's doc comment and its `MAX_WORKERS < MAX_EVENT_THREADS` const-assert describe per-worker lanes, while D2d reuses the constant as a per-type writer ceiling.

#### W-B. The K3b ratchet binary cannot be built as specified
**Where:** D14 "Rows", the standing legs, and K3b's red control (b).

**Problem:**
- **(1) `EnginePlugins` cannot be built twice in one process.** The second build panics in `register_component_hooks::<DirectionalLight>`, because component hooks are process-global and not idempotent. This is measured and recorded in seven `boyko_app` test headers (`particle_host_reachable.rs:50-56`, `profiling_host_reachable.rs:37-39`, `log_host_*.rs`). Every one of them uses one binary per host build. Rev 2 puts four rows, an "in-file canary composition" and a generator into one binary and expects "running 5 tests".
- **(2) The software rows are not gated `not(feature = "hwrt")`.** `hwrt` is compile-time (`plugins.rs:682-683`). In the `--features hwrt` leg the software rows would therefore build the hwrt composition and compare it with the software FROZEN set. The two differ by at least the 41-vs-37 `DeferredOpen` split.
- **(3) The CI guard cannot confirm the hwrt row ran.** It accepts `running [1-9]… tests`, and libtest counts `#[ignore]`d tests in that line. A software row or the ignored generator alone satisfies it.

**Consequence:**
- K3b goes red for reasons unrelated to ambiguity: the second-build panic, or the software-vs-hwrt mismatch.
- The CLAUDE.md pass criterion "running 5 tests" cannot be met by any correct layout, so the developer restructures ad hoc and the leg's text rots on arrival.

**Confidence:** CONFIRMED.

**What is needed:**
- One `EnginePlugins` build per test binary: Main and Fixed come from a single `finish()`.
- Software rows `cfg(not(hwrt))`.
- The canary either rides on the same App, with the assertion that the diff is exactly the canary pair, or runs on a bare `ScheduleBuilder`.
- Restate the expected count for each leg, and make the CI step check that the hwrt row's test name passed rather than N ≥ 1.

#### W-C. W5's "can only lose lines" leaks through RESOLVED and ALIASES
**Where:** D14 `RESOLVED`, `ALIASES`, `FROZEN_DIGEST`.

**Problem:**
- **RESOLVED.** The digest covers only FROZEN. RESOLVED is "append-only" by statement alone. When a resolved pair returns, the test's own message ("#k resolved earlier has returned") points straight at the one-integer deletion that silences it.
- **ALIASES.** The checks (`old` ∈ FROZEN, `new` live, `old` not live, bijection over the alias map) do not forbid `new` from being a name that is already pinned in FROZEN. Deleting system X and aliasing X→B, where B is an existing system, turns X's frozen pairs into cover for B's new pairs.

**Consequence:** in the refactor-last campaign, a lost edge that brings back the F1 pair (`visibility_sync × gather_mesh_draws`, once resolved) is made green by deleting its RESOLVED index. The same happens if a new B × `gather_mesh_draws` `DeferredOpen` pair is laundered through X→B. In both cases the ratchet stays green over an F1-class pair.

**Confidence:** CONFIRMED from the plan text.

**What is needed:**
- Put RESOLVED, ALIASES and ALLOWED under the same deliberate-edit guard as FROZEN (one digest over all four).
- Require the alias map to be injective over the whole FROZEN name set, i.e. `new` ∉ names(FROZEN).

### 🟢 Optional

- **O-1. D8b's "immediate" arm can never fire.**
  - Reserving an id advances `next_entity_id` at reservation time: `mint_fresh` calls `fetch_add` (`entity/entity_reservoir.rs:186-190`), and so does the batch path (`:228`).
  - A target that was reserved but not yet spawned therefore always has id < `next_fresh()` at apply.
  - Only the deferred arm (same generation, now live) works. As written, K2c test (3) passes only if it runs two applies.
  - Fix: drop the immediate arm, or key it on a watermark captured at the start of the phase.
- **O-2. Lanes are never released.** A world whose schedules are rebuilt accumulates `cap × size_of::<E>()` × 2 per writer per rebuild, and panics at build after 64 writer inits of one type. No runtime rebuild exists today (only App, demo and tests build schedules). Name this in the limits.
- **O-3. The census "count" is ambiguous.** It could mean lines or calls: `enable_chunk_byte_identity.rs:486,493,517,524` each carry two calls per line. Define which.

## Positive (keep these)

- **C1 at the type level.** `DynEnableAny`/`DynEnableIn` make an undeclared runtime term a compile error. There is no per-row cost, and the Bevy `transmute_lens` precedent fits.
- **Per-parameter open-ness built on a required const.** Non-deferring elements compile to today's code.
- **Lane 0 fixed and flattened last.** Growth appends, so no cached index ever moves. `&mut` removes a real race from safe code.
- **The frame-level obligation and the reference model with a `split` switch.** A future KE17 change gets a gate that can go red.
- **The debug view is a copy, not a pointer into the system's `Box<OrderMeta>`.** This correctly avoids aliasing under the `&mut self` of `run_dispatcher`.
- **Self-found (a) is a correct retraction.** `next_fresh`, the free-stack size and the live count are deterministic; I checked the reservoir claim protocol.

## Open questions

1. In `split_retire_guard`, does the reference model take S's duration from the configured busy-wait rather than from the recorded traces? Under the barrier S never overlaps D, so the traces contain no S-vs-D timing.
2. Which five tests does the CLAUDE.md leg expect once W-B is fixed?
