# boyko-engine

A Rust ECS engine for games, built for **ultimate performance, cache locality, and native parallelism**. No comfort-vs-speed compromises.

## Layout (branch `ecs`)

`ecs` is the active development branch — the full engine, builds green. A Cargo workspace:

- **Core:** [`boyko_ecs`](crates/boyko_ecs/) (ECS kernel: memory, components, archetypes, queries, events, scheduler, change-detection, hooks/observers, commands, serialize seam) · [`boyko_macros`](crates/boyko_macros/) (`#[derive(Component/Bundle)]`, `#[event]`) · [`boyko_utils`](crates/boyko_utils/) (`BitSet`/`BitMask`/`SparseMap`/`Slot`) · [`boyko_threadpool`](crates/boyko_threadpool/) (Chase-Lev work-stealing)
- **Std-lib / sim:** [`boyko_math`](crates/boyko_math/) · [`boyko_scene`](crates/boyko_scene/) (Transform/Camera) · [`boyko_physics`](crates/boyko_physics/) (in-house 3D TGS-Soft) · [`boyko_sdf_math`](crates/boyko_sdf_math/) · [`boyko_input`](crates/boyko_input/) · [`boyko_serialize`](crates/boyko_serialize/)
- **Render / UI:** [`boyko_rhi`](crates/boyko_rhi/) + [`boyko_rhi_vulkan`](crates/boyko_rhi_vulkan/) (in-house RHI, raw-FFI Vulkan) · [`boyko_render`](crates/boyko_render/) (GPU columns, lighting, SDF) · [`boyko_shaderdsl`](crates/boyko_shaderdsl/) (shader eDSL: one generic Rust body per leaf, instantiated over `f32` — the host oracle — and `Emit` — the HLSL printer) · [`boyko_ui`](crates/boyko_ui/) (ECS-native UI) · [`boyko_fontbake`](crates/boyko_fontbake/) (MSDF atlas) · [`boyko_image`](crates/boyko_image/) (in-house PNG/zlib/DEFLATE decoder, zero third-party deps)
- **Host / apps / bench:** [`boyko_app`](crates/boyko_app/) (host layer: OS loop + device-singleton boot + windowed runner + `EnginePlugins`) · [`boyko_demo`](crates/boyko_demo/) · [`bench_bevy_vs_boyko`](crates/bench_bevy_vs_boyko/) · [`src/main.rs`](src/main.rs) (library-shaped)

Full subsystem map → [docs/FEATURE_MAP.md](docs/FEATURE_MAP.md) (first point of contact), [docs/SYSTEMS.md](docs/SYSTEMS.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Principles (INVIOLABLE)

0. **One unified engine — `boyko_ecs` is THE SDK for logic AND data; always use it.** Every system (physics, render, input, lighting, GUI, …) is a first-class part of ONE engine — **components + systems on the ECS's own storage** — never a subsystem "glued on the side" (не прилеплено сбоку) with its own data structures or a system-local wrapper. **No parallel data system:** durable per-entity / per-element / bulk subsystem data lives in the ECS's own storage — `ComponentPool` columns, `Resource`-owned columns, or **dense (non-fragmenting) components** for the "one contiguous buffer for all instances" cases (solver state, GPU instances) — *never* `std::Vec` / `HashMap` as a side store. Logic is ECS systems on the engine's scheduler. A capability a subsystem needs is made a **first-class kernel feature** used uniformly by all systems, not a per-crate adapter. *Legitimate exceptions* (not violations): the ECS's own storage implementation; FFI / GPU / OS-contiguity buffers (Vulkan `*const T + count`, swapchain images, the OS input ring); lock-free threadpool internals; truly transient function-local scratch. "ECS-native" and "cache-optimal" are the same thing because the kernel storage (`ComponentPool` on `VmReservation`, SIMD-aligned, address-stable, per-row `row_ptr` provenance) IS the fast storage — deep integration costs no perf. *(A `std::Vec` physics mirror — a parallel data system glued on the side — caused the O11-SP4 colored-solve data race; the fix is dense components in the kernel. See [docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md](docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md), [docs/DENSE-COMPONENTS-PLAN.md](docs/DENSE-COMPONENTS-PLAN.md).)*
1. **Zero runtime overhead** — no `dyn Trait` / `Box` / `HashMap` / `Vec::new()` in the hot path without justification.
2. **Data-Oriented Design** — Struct of Arrays, hot/cold field split.
3. **Cache optimization (D-cache + I-cache)** — both levels of cache matter equally:
   - **D-cache (data)**: `#[repr(C)]` where layout matters, cache-line alignment (64 B), SoA + hot/cold field split, padding against false sharing, sequential access patterns, software prefetching for predictable patterns, non-temporal stores for streaming writes. Keep the working set of hot loops within L1d (~32 KB) / L2 (~256-512 KB) where it is critical.
   - **I-cache (instructions)**: a compact hot path, no blind `#[inline(always)]` (see principle 7), `#[cold]` / `#[inline(never)]` for error paths and rare branches, controlled branch density, minimized hot-loop size. PGO (`-Cprofile-use=...`) is applied when an execution profile exists.
4. **Lock-free parallelism** — no `Mutex` / `RwLock` / `RefCell` on the hot path.
5. **Minimum allocations** — preallocate during setup, reuse during gameplay.
6. **SIMD-friendly layout** — data ready for vectorization.
7. **Measured inlining** — `#[inline]` for trivial cross-crate and generic methods (otherwise the body is invisible to LTO). `#[inline(always)]` ONLY when a profiler or assembly inspection has shown that the compiler is not inlining on its own and that this matters. `#[cold]` / `#[inline(never)]` for error paths and rare branches. Excessive inlining bloats the L1i cache and **lowers** performance — decisions must be measurement-driven, not doctrine-driven.
8. **Unsafe is justified** — but **every** `unsafe` block carries a `// SAFETY:` comment stating the invariants.

## Build commands

```powershell
cargo check --workspace --all-targets                            # fast type check
cargo build --release                                            # release build
cargo clippy --workspace --all-targets -- -D warnings            # linter
cargo test --workspace --all-targets --no-fail-fast              # tests
cargo bench                                                      # benchmarks
cargo +nightly miri test                                         # UB detector (if nightly is installed)
```

⚠️ **`--workspace` and `--no-fail-fast` are both load-bearing, and each was added after a
measurement, not for tidiness.**

- **`--workspace`**: without it the virtual manifest at the repo root type-checks a subset and
  reports success — the 2026-07-23 audit found the whole CI vacuum-green this way.
- **`--no-fail-fast`**: `cargo test` **stops at the first failing target**, so one known-red target
  *shadows* every target ordered behind it. MEASURED 2026-08-10: the workspace had **three** red
  targets while every report said "green except the known `internal_docs_anchors`" — the trybuild
  fixture `token_use_after_submit_rejected` had been red for 87 commits (a line added, its `.stderr`
  never re-blessed) and `cluster_bound_arraylength`'s shader set had been stale since VG R3's batch
  cull gained a bound query. Neither was visible until the flag was passed. **"Green" without this
  flag means "green up to the first thing already known to be red".**

### The ignored suite — legs by what the machine has

The four commands above run **none** of the `#[ignore]`d tests — **311 sites (172 unconditional +
139 `#[cfg_attr(<cfg>, ignore = …)]`), measured 2026-09-18 on this line.** The move from
2026-09-17's 280 is two things and not one: the A6 lane's three new test files brought **32**
`cfg_attr` sites (`a6_schedule_panic_propagation.rs` 17, `a6_panic_propagation.rs` 14,
`a6_panicked_scope_chunk_receipts.rs` 1, all `cfg_attr(miri, …)`), and the census stopped counting
**two phantoms** — lines that BEGIN inside a `\`-continued string in a `panic!` and merely start
with `#[cfg_attr(`; one of them was inside the 280, the other inside the 313 this merge first
printed. Every one of them now states its requirement at the site, and
[tests/ignore_reasons_census.rs](tests/ignore_reasons_census.rs)
fails the build if a new one does not — a bare `#[ignore]` is the **third** way to make a check
disappear, after `unsafe` and `#[allow(clippy::disallowed_types)]`, and it now carries a written
rationale like the other two.

⚠️ **Every count in this section is a snapshot and drifts upward with each campaign; the census
prints the current figure on every run** (`cargo test -p boyko-engine --test ignore_reasons_census
-- --nocapture` → `[ignore census] N sites (P plain, C cfg_attr) across … .rs files walked`). Its
`MIN_SITES` is a floor, so a prose count here may drift arbitrarily far and stay green — read the
run, not this paragraph. The figures below stood at 164 (143 + 21) when the section was written and
were already stale before this line was added.

**Leg: device-free ignored tests.** Runs on any machine, no GPU, ~0.03 s. **Covers 1 test.**

```powershell
cargo test -p boyko-log --test l14_sink_policy -- --ignored --test-threads=1
```

The output must read `running 1 test`. A `running 0 tests` line is a vacuous pass, not a pass —
see the `#![cfg(miri)]` trap below, which produced exactly that.

**Leg: device-needing ignored tests.** A machine with the GPU; the orchestrator or the owner runs
it, per-binary with `--test-threads=1`. It covered **135 tests** when it was written, across
`boyko_app`, `boyko_render` and `boyko_rhi_vulkan`; **that count has NOT been re-taken on this
line**, and it cannot be re-derived from the reason strings (see the ⚠ below), so it is left as the
historical figure rather than guessed forward. What IS mechanical today: those three crates hold
**145 of the 172 plain sites** (`boyko_app` 88, `boyko_rhi_vulkan` 51, `boyko_render` 6, measured
2026-09-17), which is the population this leg is drawn from, not the leg itself. Four of them need a
non-default cargo feature and are not even *compiled* otherwise (`--features hwrt` ×3,
`--features spec_constant_smoke` ×1); most of the rest additionally sit behind `#![cfg(windows)]`
and vanish on Linux. There is no single command — each binary has its own env-var protocol in its
module header (`BOYKO_DISABLE_VALIDATION`, `BOYKO_HZB_DUMP`, `BOYKO_WINDOW_FRAMES`, …).

**Leg: Miri.** `cargo +nightly miri test` already carries **138** of the ignores (measured
2026-09-18 on this line) — the **137** `cfg_attr` sites whose cfg is `miri` (134) or
`any(miri, debug_assertions)` (3), plus `miri_fixed_loop`'s one plain ignore. The `miri` sites run
*natively* in both profiles and are skipped only under Miri; the `any(miri, debug_assertions)`
sites run natively in RELEASE only — their leg is the physics release run below, and after the A7
lane there are five of them, not three. All 32 of the sites added since 2026-09-17 landed here,
which is why this figure moved and the two above it did not. The two remaining `cfg_attr` sites are
not Miri's: `profiling/reduce.rs`'s
`not(debug_assertions)` and `tb_neg_m2w_block_reference.rs`'s `not(all(miri, feature = …))`. None of
the 138 belong to either leg above.

**Leg: physics release — the debug-ignored `slow:` tests.** Five tests are ignored in every debug
build and under Miri by `#[cfg_attr(any(miri, debug_assertions), ignore = "slow: …")]`, so none of
the four commands above runs them: G2, G6, G7, A7-R1 and A7-R2 in
[`crates/boyko_physics/tests/sleep_settles_box_piles.rs`](crates/boyko_physics/tests/sleep_settles_box_piles.rs)
— box piles through the real physics schedule, A7-R1 being the only scene-level gate on a resting
pile's creep (defect A7). Their leg is the physics release run:

```powershell
cargo test --release -p boyko-physics --no-fail-fast
```

In it `sleep_settles_box_piles` must print `running 12 tests` and `11 passed; 0 failed; 1 ignored`
(the one ignore is its `generator:`); that binary takes ~80 s, ~73 s of it A7-R1 (msvc,
2026-09-18). `-- --ignored` in a debug build is NOT their leg: the debug build is exactly where
they are ignored, and it would run them unoptimized.

**At least 28 ignored tests belong to no leg at all**, and must not be swept into one. Six were
enumerated when this section was written: three *generators* that assert nothing and emit source to
paste (`dump_maximal_frame_barrier_stream`, `dump_vb_unsplit_barrier_streams`,
`dump_vb_split_barrier_streams` — the second's own doc warns that running it casually re-measures
the baselines the split is compared against), two *deferred milestones* that are RED by design until
M2's JCGT cubic lands (`brick_field_is_conservative_lower_bound`,
`trilinear_reconstruct_is_a_tight_lower_bound_in_r1`), and one *timing probe* documented "NOT a CI
gate" (`no_starvation_every_worker_makes_progress`). None of those six carries a vocabulary prefix.
**A further 22 sites do carry one** (21 `deferred:`, 1 `generator:`, measured 2026-09-17), in
`boyko_ecs` (9), `boyko_serialize` (5), `boyko_physics` (6 — five of them the A4/A7 lane's), and
`boyko_ui` (2); each is red or silent by design and belongs to no routine leg either. 28 is a floor,
not a census: the 147 plain reasons that carry no prefix have not been classified.

⚠️ **The device-free leg is one test, and it is an explicit invocation rather than a filter,
because the partition CANNOT be derived from the reason strings.** A keyword classifier over
`{GPU, RTX, Vulkan, windowed, device, dispatch}` put 10 on the device-free side — of the 143 plain
sites the tree held when the experiment was run, 172 today — and **8 of those 10 are wrong**, wrong
in the direction that produces a green:

- `negative_chained_barrier_hazard` and `a5_gpu_off_vs_on_wall_clock_ab` **do** need a device; their
  reasons name the *hazard* and the *purpose*, not the requirement. Both call `boot_*_or_skip`, so
  on a GPU-less box they return early and **pass** — a leg that includes them reports green while
  measuring nothing.
- `miri_app_driver_substeps_and_hold` is inside a `#![cfg(miri)]` file, so natively it does not
  exist. Filtering for it prints `running 0 tests` and exits 0. **Measured while writing this leg.**
- The other five are the generators / deferred / flaky above, which would pass meaninglessly, fail
  outright, or flake.

The property the leg needs — *what does this test require, and is it a gate at all* — is simply not
what a prose reason answers. **Making it mechanical takes a reason PREFIX from a closed vocabulary**,
checked by the same census: `#[ignore = "<class>: <prose>"]` with `class` one of `gpu`,
`gpu-windowed`, `gpu-cap` (RT / ray-query / `VK_KHR_pipeline_executable_properties`), `feature`,
`solo` (device-free, needs `--test-threads=1`), `slow` (device-free, wall-clock budget),
`miri-slow`, `miri-unsupported` (Miri cannot execute what the test needs AT ALL — a child process,
a custom `#[global_allocator]` — where `miri-slow` means it would finish, given time; 6 sites, the
A6 lane's, all `cfg_attr(miri, …)`), `generator`, `deferred`, `flaky`. **The claim that the tree
maps onto it "exactly" was true of a 143-site tree and is not true now:** that mapping (135
`gpu*`/`feature`, 1 `solo`, 1 `miri-slow`, 3 `generator`, 2 `deferred`, 1 `flaky`) sums to 143
against **172** plain sites today.
The migration has started at the sites, not in this list: **25 of the 172 plain reasons already
carry a prefix** — 21 `deferred:`, 2 `gpu-windowed:`, 1 `slow:`, 1 `generator:` (measured
2026-09-17; at HEAD 17 / 2 / 1 / 0, so the A4/A7 lane's five are the newest sample) — and the other
147 do not. So the migration is still mechanical *per site*, and afterwards each leg is a `grep` and
every new ignore picks its own leg at the site; what it is not is bookkeeping already done.

⚠️ **One prefix is already being used against its own definition, and a mechanical leg built from
it would run those tests in the wrong place.** `slow` is defined above as *device-free, wall-clock
budget*, i.e. a `--ignored` leg — but `sleep_settles_box_piles.rs`'s G2, G6 and G7 spell
`#[cfg_attr(any(miri, debug_assertions), ignore = "slow: …")]`, and that file's module header says
the opposite for them: they run in the ordinary **release** test run, and `-- --ignored` in a debug
build is NOT their leg. Harmless while the census only enforces non-emptiness; it bites the day the
partition becomes a `grep`. Whichever ruling makes the prefixes mechanical has to say which class a
release-only-but-device-free test takes.

## Target platform

- OS: Windows / Linux (x86_64)
- SIMD: AVX2 baseline; AVX-512 optionally via `cfg(target_feature)`
- Edition: Rust 2024

## Documentation — two layers

- **Internal (for agents):** [docs/FEATURE_MAP.md](docs/FEATURE_MAP.md) (**first point of contact** — "where is X?"), [docs/SYSTEMS.md](docs/SYSTEMS.md) (subsystem catalog + file:line), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (layers/deps/data-flow). Consult before starting work.
- **Public (mdBook + cargo doc → GitHub Pages):** [book.toml](book.toml) / [book/src/](book/src/) (register new pages in [book/src/SUMMARY.md](book/src/SUMMARY.md)); CI [.github/workflows/docs.yml](.github/workflows/docs.yml) deploys to `https://bluesteelll.github.io/boyko-engine/` (+ `/api/` rustdoc). Written ONLY by the `doc-writer` agent (others do not edit it). Local preview: `mdbook serve --open`.

## Agents

In [.claude/agents/](.claude/agents/) the following are defined:

| Agent | Purpose |
|-------|---------|
| `architect` | Designs the architecture of new features |
| `researcher` | Collects practices from Bevy / flecs / EnTT / Unity DOTS |
| `architecture-critic` | Critiques the plan before implementation |
| `developer` | Implements code following the plan |
| `code-reviewer` | Reviews the written code |
| `tester` | Build + unit / integration / proptest / loom + criterion |
| `results-analyst` | Final verdict after the feature is implemented |
| `project-analyst` | Free-form codebase analysis, security audit, Q&A |
| `doc-writer` | Writes public documentation in `book/src/` for the GitHub Pages deploy |

The main Claude in the chat acts as the **orchestrator** — chooses the right agents for each task and runs the iteration loops.

**Model routing** (set via each agent's `model:` frontmatter): **every role runs on Opus** — owner decision, 2026-07-26. The previous split put `developer`, `tester`, `researcher`, `doc-writer` and `project-analyst` on Sonnet as "mechanical / gathering" roles. That premise did not survive contact with this codebase: the roles it called mechanical are the ones that catch the campaign's defects. On the VB-SV0 stage alone, implementers refuted the orchestrator's own prescriptions **nine times** — a ULP tolerance that was wrong in form because the leaf ends in a cancellation, a `-D`-push route replaced by an `OpArrayLength` bound the orchestrator had not considered, a z-range check whose prescribed site would have panicked at boot in every process, and a NaN claim corrected on the wrong leaf. None of that is transcription work. Do not downgrade any role without a measured before/after quality diff — this applies to all nine now, not only to `code-reviewer` (which remains the last line against `unsafe`/atomics UB). The orchestrator itself stays on the session model.

## Orchestration discipline

- **Clarify before acting.** If a request is ambiguous, or you do not fully understand the intended scope/behavior, **ask** (`AskUserQuestion`) or **enter Plan Mode BEFORE any Write/Edit** — never guess. Only VALUES/SCOPE calls go to the owner; decide perf/architecture forks yourself, with numbers.
- **Plan-Mode threshold.** Any change touching **≥3 files**, or that you cannot describe in one sentence, goes through Plan Mode (`ExitPlanMode` approval) before the first edit.
- **Backstop hook.** A `UserPromptSubmit` hook ([.claude/hooks/clarify_gate.py](.claude/hooks/clarify_gate.py)) injects a reminder when a prompt is imperative but names no file/path/symbol. It reminds; it never blocks. Subagents cannot ask the user — a subagent that hits an ambiguity **stops and escalates to the orchestrator** (already encoded in `developer.md`).
- **graphify-first, one retry.** The PreToolUse hooks ([graphify_read_gate.py](.claude/hooks/graphify_read_gate.py), [graphify_bash_gate.py](.claude/hooks/graphify_bash_gate.py)) nudge `graphify query/explain/path` before reading/grepping source. graphify is tuned to the ECS kernel; if it returns off-target or empty results, fall back to Grep/Read **once** — do not retry graphify.

## Communication

- Chat messages between Claude and the user can be in Russian.
- **Every artifact written into the repository is in English**: code, doc comments, inline comments, commit messages, internal docs, agent prompts, mdBook content, audit reports — everything. No mixed-language files.
- **ONE EXCEPTION: [`docs/ru/`](docs/ru/).** Owner-granted 2026-08-02. That directory holds Russian versions of documents the owner reads and edits himself. The files there are NOT a rule violation and must not be "fixed" back to English. Everything outside it stays English, including the originals. The English version is the SOURCE OF TRUTH and the Russian one follows it; editing either side updates the other **in the same commit**, because a diverged pair is worse than a missing one — the reader cannot tell which is current and finds out only by acting on the stale one. See [`docs/ru/README.md`](docs/ru/README.md).

## Rules for agents

### Forbidden on the hot path
- `Box<dyn Trait>`, `Rc`, `Arc<Mutex<_>>`
- `HashMap` (use an array indexed by `ComponentId` instead)
- `Vec::new()`, `format!()`, `String::from()` (preallocate everything)
- `clone()` of large structs
- Virtual dispatch

**Mechanically enforced** (2026-07 audit): [`clippy.toml`](clippy.toml)'s `disallowed-types`
fails the existing `cargo clippy --all-targets -- -D warnings` gate on `HashMap`/`HashSet`/
`Mutex`/`RwLock`/`Rc`. A legitimate exception (once-per-type `TypeId` mint registry, setup /
load-time structure, boot plumbing, `#[cfg(test)]` oracle model) carries an explicit
`#[allow(clippy::disallowed_types)]` **plus a rationale comment** — one grep enumerates every
exception, exactly like the mandatory `// SAFETY:` comments.

### Shaders
HLSL that the eDSL owns is **generated, never hand-edited**: extend
[`boyko_shaderdsl`](crates/boyko_shaderdsl/), re-emit, re-splice between the
`// === GENERATED <name> BEGIN/END ===` sentinels, and let the `*_edsl_sync` tests re-run the
generator and pin the result. Compilation is offline+hermetic (the frozen `dxc` recipe in each
shader's header); the committed `.spv` are byte-gated by the `*_spv_sync`/`*_edsl_sync` re-DXC
tests. One source may compile to N `.spv` via `-D` — every variant gets a row in
[docs/SHADER-VARIANT-MANIFEST.md](docs/SHADER-VARIANT-MANIFEST.md).

### Required for every `unsafe`
```rust
// SAFETY: <concrete invariants that guarantee correctness>
unsafe { ... }
```

### Separation of duties
- `developer` writes code but **does not run tests** — that is the `tester`'s job.
- `code-reviewer` finds issues but **does not fix code** — that is the `developer`'s job.
- `architecture-critic` critiques the plan but **does not dictate the design** — that is the `architect`'s job.
- `project-analyst` answers questions but **does not edit** anything.

### Git
- Never commit without an explicit user request.
- Never use `--force` / `--no-verify` without explicit permission.
- Commits are authored only by the repository owner. **Never** add `Co-Authored-By: Claude ...` (or any equivalent AI-assistant marker) to commit messages. The history must read as the author's own work.

## Code conventions

- **Naming**: `snake_case` (functions / variables), `CamelCase` (types / traits), `SCREAMING_SNAKE_CASE` (constants).
- **Doc comments** (`///`) on every public item.
- **Comments explain "why", not "what"** — no `// increment counter` above `x += 1`.
- **`expect("invariant: ...")`** instead of `unwrap()` wherever panic is by design.
- **`debug_assert!`** for invariant checks on the hot path (they vanish in release).
- **Imports grouped**: std → external → crate → self.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
