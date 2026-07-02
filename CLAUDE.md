# boyko-engine

A Rust ECS engine for games, built for **ultimate performance, cache locality, and native parallelism**. No comfort-vs-speed compromises.

## Layout (branch `ecs`)

`ecs` is the active development branch — the full engine, builds green. A Cargo workspace:

- **Core:** [`boyko_ecs`](crates/boyko_ecs/) (ECS kernel: memory, components, archetypes, queries, events, scheduler, change-detection, hooks/observers, commands, serialize seam) · [`boyko_macros`](crates/boyko_macros/) (`#[derive(Component/Bundle)]`, `#[event]`) · [`boyko_utils`](crates/boyko_utils/) (`BitSet`/`BitMask`/`SparseMap`/`Slot`) · [`boyko_threadpool`](crates/boyko_threadpool/) (Chase-Lev work-stealing)
- **Std-lib / sim:** [`boyko_math`](crates/boyko_math/) · [`boyko_scene`](crates/boyko_scene/) (Transform/Camera) · [`boyko_physics`](crates/boyko_physics/) (in-house 3D TGS-Soft) · [`boyko_sdf_math`](crates/boyko_sdf_math/) · [`boyko_input`](crates/boyko_input/) · [`boyko_serialize`](crates/boyko_serialize/)
- **Render / UI:** [`boyko_rhi`](crates/boyko_rhi/) + [`boyko_rhi_vulkan`](crates/boyko_rhi_vulkan/) (in-house RHI, raw-FFI Vulkan) · [`boyko_render`](crates/boyko_render/) (GPU columns, lighting, SDF) · [`boyko_ui`](crates/boyko_ui/) (ECS-native UI) · [`boyko_fontbake`](crates/boyko_fontbake/) (MSDF atlas)
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
cargo check --all-targets                          # fast type check
cargo build --release                              # release build
cargo clippy --all-targets -- -D warnings          # linter
cargo test --all-targets                           # tests
cargo bench                                        # benchmarks
cargo +nightly miri test                           # UB detector (if nightly is installed)
```

## Target platform

- OS: Windows / Linux (x86_64)
- SIMD: AVX2 baseline; AVX-512 optionally via `cfg(target_feature)`
- Edition: Rust 2024

## `master` vs `ecs` (history)

`master` = memory subsystem only (generic `ComponentPool<T>`/`Chunk<T>`, two-level `UnitId` addressing). `ecs` (here) = the full architecture: type-erased `ComponentPool`/`Chunk` (Layout via `ComponentRegistry`), direct-pointer `Unit`, `usize` ids, `EntityMaster` recycling, global metadata registries, `EcsMaster` facade. Full detail → [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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

## Communication

- Chat messages between Claude and the user can be in Russian.
- **Every artifact written into the repository is in English**: code, doc comments, inline comments, commit messages, internal docs, agent prompts, mdBook content, audit reports — everything. No mixed-language files.

## Rules for agents

### Forbidden on the hot path
- `Box<dyn Trait>`, `Rc`, `Arc<Mutex<_>>`
- `HashMap` (use an array indexed by `ComponentId` instead)
- `Vec::new()`, `format!()`, `String::from()` (preallocate everything)
- `clone()` of large structs
- Virtual dispatch

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
