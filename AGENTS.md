# boyko-engine

A Rust ECS engine for games, built for **ultimate performance, cache locality, and native parallelism**. No comfort-vs-speed compromises.

**Orchestrator: read [docs/WORKFLOW.md](docs/WORKFLOW.md) and `D:/tmp/phys-orch/RESUME.md` (the live campaign checkpoint, rewritten at every stop) before launching lanes.**

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
cargo +nightly-x86_64-pc-windows-msvc miri test                  # UB detector (if nightly is installed)
```

⚠️ **`--workspace` and `--no-fail-fast` are both load-bearing:** without `--workspace` the virtual
root manifest type-checks a subset and reports success; without `--no-fail-fast` `cargo test` stops at
the first failing target, so one known-red target shadows every target behind it — **"green" without
it means "green up to the first thing already known to be red".**
(rationale and measurements: docs/WORKFLOW.md#build-command-flags)

**On the workstation, a gate run sets `TMP`/`TEMP` to `D:/wt/_targets/tmp` first** (RK-18 of the
unification plan), and the directory must exist before cargo starts — `link.exe` writes its own
temporaries there, and a missing path is `LINK : fatal error LNK1104`:

```powershell
$env:TMP = 'D:/wt/_targets/tmp'; $env:TEMP = $env:TMP; New-Item -ItemType Directory -Force $env:TMP | Out-Null
```
```bash
mkdir -p D:/wt/_targets/tmp && export TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp
```
(rationale and measurements: docs/WORKFLOW.md#gate-temp-directory)

**`Cargo.lock` is tracked (2026-09-24), and CI runs every cargo build with `--locked`.** A worktree
created before that commit holds an untracked `Cargo.lock`: move it aside before merging a commit that
tracks it. (rationale: docs/WORKFLOW.md#lockfile)

### The feature axis — `--all-targets` is not `--all-features`

None of the commands above compiles a `#[cfg(feature = "…")]` item, and `--all-features` cannot work
in this tree (`boyko-threadpool/tb-neg-m2w` is a `compile_error!` outside Miri; three `nightly`
features are E0554 on stable). Features are checked per crate, one leg each: CI's `feature-legs` job
derives its legs from `cargo metadata` minus a refusal table whose rows state their measured reason.
Locally:

```powershell
cargo check -p boyko_rhi_vulkan --features hwrt --all-targets    # the leg that was red for 32 days
cargo check -p boyko-app        --features hwrt --all-targets    # its cfg(windows) arm: no CI job
cargo check -p boyko-render     --features hwrt --all-targets    # the example's cfg(windows) loop
```

- **Known RED, refused by name in `ci.yml`:** `boyko_shaderdsl/nightly` (and `boyko_sdf_math/nightly`,
  which forwards to it), broken on both channels; `boyko-app/profiling-alloc`, whose `#[global_allocator]`
  collides with the one in `crates/boyko_app/tests/e1_engine_ui_alloc_census.rs:271`.
- **The platform cfg has the same blind spot, narrowed.** `boyko-app` compiles on Linux; no GPU path:
  `gpu_scene`, the host and the frame loop are `#[cfg(windows)]` (`crates/boyko_app/src/lib.rs:60-115`,
  `runner.rs:237-238`); the non-Windows `run_windowed` reports E3004 and exits (`runner.rs:1022-1026`).
  Every CI job runs on `ubuntu-latest`, so no job compiles a `#[cfg(windows)]` item.
- No clippy lint has run over feature-gated code, and the `#[cfg(windows)]` arms of `boyko-app` and of
  `orbit_cube_window` get only the by-hand recipe above, which is a recipe, not coverage.

(rationale and measurements: docs/WORKFLOW.md#feature-axis)

### The ignored suite — legs by what the machine has

`cargo test` runs **none** of the `#[ignore]`d tests. Every site reads `#[ignore = "<class>: <prose>"]`,
and [tests/ignore_reasons_census.rs](tests/ignore_reasons_census.rs) fails the build on a missing or
unknown class, or on one the site's own source contradicts (enforced since rung B3, 2026-09-23).
Quote the census's printed lines, never a prose count:
`cargo test -p boyko-engine --test ignore_reasons_census -- --nocapture`.

- **Classes** (closed list): `gpu`, `gpu-windowed` (device + window + swapchain), `gpu-cap` (RT / ray
  query / `VK_KHR_pipeline_executable_properties`), `feature` (names its `--features <name>`), `solo`
  (device-free, `--test-threads=1`), `slow` (device-free, wall-clock budget), `miri-slow`,
  `miri-unsupported`, and the no-leg `generator`, `deferred`, `flaky`. The only composites are
  `feature+<class>` and `gpu-windowed+gpu-cap`. One site is exempt: it is ignored only in release and
  names no leg (`boyko_app/src/profiling/reduce.rs`).
- **Device-free** (any machine): `rg -n '^[^/]*#\[ignore = "(solo|slow):' -g '*.rs'`; each reason names
  its command. The two `solo` sites:
  `cargo test -p boyko-log --test l14_sink_policy -- --ignored --test-threads=1` and
  `cargo test -p boyko-ecs --lib ecs::memory::component_pool::tests::commit_floor_tests::os_truth_row1_default_pool_at_full_capacity -- --ignored --exact --test-threads=1`.
  Select by name (`--exact`) where a binary also holds a no-leg class. Each run must print
  `running N tests` for the N selected; **`running 0 tests` is a vacuous pass, not a pass.**
- **Device-needing** (every class with a `gpu` part; the orchestrator or the owner):
  `rg -n '^[^/]*ignore = "(feature\+)?gpu' -g '*.rs'`; per binary, `--test-threads=1`, env vars per its
  module header (`BOYKO_DISABLE_VALIDATION`, `BOYKO_WINDOW_FRAMES`, …). A `feature+` site is not even
  compiled without its flag (`--features hwrt` / `spec_constant_smoke`); most device sites are
  `#![cfg(windows)]`.
- **Physics release:** `cargo test --release -p boyko-physics --no-fail-fast`. The six
  `cfg_attr(any(miri, debug_assertions), ignore = "slow: …")` tests in
  `crates/boyko_physics/tests/sleep_settles_box_piles.rs` run only here (`running 13 tests`,
  `12 passed; 0 failed; 1 ignored`); `-- --ignored` in a debug build is NOT their leg.
- **Miri:** `cargo +nightly-x86_64-pc-windows-msvc miri test` skips every `miri-slow` /
  `miri-unsupported` site (they run natively). The two Miri-only tests (`miri_fixed_loop.rs`,
  `miri_phase19.rs`'s `miri_cascade_wide_path`) run under Miri with `-- --ignored`; the tb-neg arm runs
  only through `scripts/tb_neg_gate.sh`.
- **No leg — never sweep them into one:** `generator`, `deferred`, `flaky`.
- **Never pick a leg by filtering reason strings:** a keyword classifier misfiled 8 of 10 sites, every
  one toward a false green; the prefix is the partition.

(rationale and measurements: docs/WORKFLOW.md#ignored-suite)

## Target platform

- OS: Windows / Linux (x86_64)
- SIMD: AVX2 baseline; AVX-512 optionally via `cfg(target_feature)`
- Edition: Rust 2024
- Development host (the owner's machine): **MSVC** since 2026-09-17 — `stable-x86_64-pc-windows-msvc`
  for builds, `nightly-x86_64-pc-windows-msvc` for Miri; spell both. `stable-x86_64-pc-windows-gnu`
  stays installed only to compare against numbers pinned before that date, which were blessed under
  windows-gnu. Loom's `--cfg loom` goes through a `target."cfg(windows)"` key, never
  `build.rustflags` (see the tester's instructions for why that form compiles every model away).

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

**Model routing** — the `model:` frontmatter of all nine agents stays `opus`; the lane scripts choose
the tier per call (owner, "both steps", 2026-09-30: rulings 5 and 7 under "After window 9a
(2026-09-30)" in [`docs/physics/perf-campaign/levers/00-RULINGS.md`](docs/physics/perf-campaign/levers/00-RULINGS.md)).

- **`tester`: Sonnet 5.5** (lanes started after 2026-09-30); its GREEN is still crossed by the Opus
  `code-reviewer` running beside it.
- **`developer`: Opus** for kernel, `unsafe` and numerics work; **Sonnet 5.5** for mechanical work
  (re-pins by rule, doc and registry merges, window records).
- **Sonnet 5.5 also takes** scouting and inventory, window preparation, transcription of finished
  results into docs, and re-runs of existing gates — each checked by an Opus verifier or a hard oracle.
- **Opus:** `architect`, `architecture-critic`, `code-reviewer` (the last line against
  `unsafe`/atomics UB), `results-analyst` and every verifier; `researcher`, `project-analyst` and
  `doc-writer` keep Opus. The orchestrator stays on the session model.
- **`effort: low` on Opus is NOT used** — the worst arm on the retrieval benchmark (4 confidently
  wrong, against Sonnet 5.5's 1).
- **Escape-rate watch:** a Sonnet tester GREEN later overturned by review or a window is logged; one
  escape Opus would have caught reopens this routing.
- **A tier change needs a measured diff** (≥ ~25 items, *confident-but-wrong* scored apart from
  *missed*); never downgrade a role because it is "mechanical" (tested here and false). Optimise batch
  size and redundant arms instead of the tier.
(rationale and measurements: docs/WORKFLOW.md#model-routing)

## Orchestration discipline

- **Clarify before acting.** If a request is ambiguous, or you do not fully understand the intended scope/behavior, **ask** (`AskUserQuestion`) or **enter Plan Mode BEFORE any Write/Edit** — never guess. Only VALUES/SCOPE calls go to the owner; decide perf/architecture forks yourself, with numbers.
- **Plan-Mode threshold.** Any change touching **≥3 files**, or that you cannot describe in one sentence, goes through Plan Mode (`ExitPlanMode` approval) before the first edit.
- **Backstop hook.** A `UserPromptSubmit` hook ([.claude/hooks/clarify_gate.py](.claude/hooks/clarify_gate.py)) injects a reminder when a prompt is imperative but names no file/path/symbol. It reminds; it never blocks. Subagents cannot ask the user — a subagent that hits an ambiguity **stops and escalates to the orchestrator** (already encoded in `developer.md`).
- **No code-graph tool.** graphify was removed 2026-10-07 (owner): 220 uses against 33,981 hook nudges (~1.5 % of all agent spend). Navigate with the LSP, ripgrep and `semble` (see "Code navigation"); do not reinstall a code-graph tool or a hook that mandates one.

## Token economy in workflows

Cost ≈ turns × context: most spend is re-reading context that grows inside ONE agent, not the
number of agents.
(rationale and measurements: docs/WORKFLOW.md#token-economy)

- **Do not merge stages to save agents:** a merged agent re-reads the earlier stage every later turn,
  and merging kills independent refutation (tester ∥ reviewer, triage → fix). Run parallel arms when
  their inputs or roles differ (they buy wall time, not tokens).
- **One context pack for parallel checkers:** when ≥ 2 agents of a run need the same orientation, a
  scout writes a ≤ 15k-token map (files + line ranges, key facts, the delta) to disk and every brief
  points at it; the checkers stay separate agents.
- **Keep the cache warm:** subagents use the 1-h TTL (`subagentPromptCacheTtl`, `.claude/settings.json`).
  Run a long gate detached in its own console (`Start-Process … -WindowStyle Minimized`; never
  `run_in_background`/`&` — a reaped shell's children die 0xC0000142), writing PID / DONE / exit codes /
  a per-row summary; wait in bounded slices (PID alive, within budget); past budget is a hang — stop it
  by its own PID.
- **Logs live on disk, not in context** (an output is re-read every later turn): `rg -n` + ranged reads
  (≤ ~200 lines); read summaries, verdicts, failure excerpts — never whole logs. Check `test result:`
  targets BY NAME against the expected set (equal counts over different sets are a vacuous green).
- **Hand off through files:** briefs carry paths + section names, never pasted documents; every report
  is saved; a resume = a commit + a ≤ 1-page `handoff.md`. Split a long agent only at a commit, past
  ~350k context.
- **One owner per heavy gate per round** — developer: check/clippy, its red-firsts, the gates its diff
  touches; tester: workspace tests, pins, Miri, anchors.
- **Rounds ≥ 2 are delta-scoped:** the confirmed items' red-first on parent and HEAD, ≥ 2 own mutations
  at the fix delta and at the gates the last round added, the touched gates, workspace check/clippy;
  one full workspace run on the lane's final HEAD before merge.
- **Keep every checking arm** (critique, triage, tester mutations, review each round — they catch what
  greens miss); cut one only on a measured escape-rate diff over ≥ ~25 items. Lower effort was
  measured and rejected.
- **Small always-loaded floor** (CLAUDE.md / MEMORY.md are paid every turn of every agent): MEMORY.md
  ≤ ~15 KB, measurement tables live in docs; prefer typed agents (smaller start context).
- **Orchestrator:** workflows return verdicts + paths (≤ ~2k tokens), not full agent texts; read
  reports by section; restart from the memory checkpoint past ~400k context.

## Communication

- Chat messages between Claude and the user can be in Russian.
- **Every artifact written into the repository is in English**: code, doc comments, inline comments, commit messages, internal docs, agent prompts, mdBook content, audit reports — everything. No mixed-language files.
- **ONE EXCEPTION: [`docs/ru/`](docs/ru/).** Owner-granted 2026-08-02. That directory holds Russian versions of documents the owner reads himself. The files there are NOT a rule violation and must not be "fixed" back to English. Everything outside it stays English, including the originals.
- ⚠️ **`docs/ru/` is FROZEN as of 2026-09-07 — owner decision — and its pairing rule is WITHDRAWN.** The English documents are the source of truth and are the only side maintained. Do not update the Russian side when the English one changes, and do not translate new documents into it. The pair will diverge — accepted, and announced in [`docs/ru/README.md`](docs/ru/README.md).
  - A future session must NOT "repair" the divergence by re-syncing or by deleting the directory. Either is an owner call, not a tidy-up.
  - `docs/ru/README.md` keeps its section on twin checks testing SAMENESS rather than TRUTH (`d272e1fd`,
    2026-08-29), below the freeze announcement, as the A8 merge (the `integ/unified` cut) left it;
    re-syncing or dropping it is the owner's call. A report about one file of this directory is not a
    report about the directory.

(rationale and measurements: docs/WORKFLOW.md#russian-docs-freeze)

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

## Code navigation — the LSP is the ground truth about a symbol

`rust-analyzer` serves every `.rs` file through the `LSP` tool (plugin
`~/.claude/skills/rust-analyzer-local/.lsp.json`) out of the compiler front-end, not a stale index.
Prefer it for a symbol's identity or relations: `goToDefinition` / `findReferences` /
`goToImplementation`, `hover` (incl. the **computed** layout), `workspaceSymbol`,
`incomingCalls` / `outgoingCalls`.

⚠️ **It covers ONLY the projects in `settings["rust-analyzer"].linkedProjects`, which REPLACES
auto-discovery:**

- An unlisted worktree gets **syntax only** — `hover` / `findReferences` / `workspaceSymbol` return
  **empty**, a silent wrong answer indistinguishable from "no such symbol".
- Never drop the main checkout from the list; add the lanes under work, remove each when it finishes
  (several GB RSS each). With two trees linked, `workspaceSymbol` hits once per tree: a lane answers
  with an absolute path, the main checkout with a relative one.

(rationale and measurements: docs/WORKFLOW.md#rust-analyzer)

Use `Grep` (ripgrep) for literal or pattern-shaped questions (a `// SAFETY:` census, an
`#[allow(clippy::disallowed_types)]` sweep, a shader string) and `semble` for meaning in prose.

### Meaning in prose and comments — `semble`

`semble` (machine-local, `pip install --user "semble[mcp]"`) indexes `.md` and `.rs` incl. comments
and re-validates its cache on every search.

- **Use the wrapper, not bare `semble`:** `python tools/semsearch.py "<question>" <tree> -k 8` — it
  adds the cross-encoder rerank (most of the measured value) and prints both orders (`was=#N`;
  `--no-rerank` exists). Bare `semble search` searches **code only** unless given `--content all`.
- **Do not change the wrapper's settings (chunk 4500, pool 40, fusion α=0.5) or the embedding model
  (`potion-code-16M-v2`)** — each was measured.
- **Grep vs semble:** if a keyword guess would return few files, grep; if hundreds, let semble rank.
- **A first-stage top score below ~0.012 means "nothing really matched"** — prefer a ripgrep guess
  on a rare word from the question.
- `semble mcp` does not exist (exits 2); `wgsl` / `hlsl` have no grammar (line chunking).

(rationale and measurements: docs/WORKFLOW.md#semble)
