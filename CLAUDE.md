# boyko-engine

A Rust ECS engine for games, built for **ultimate performance, cache locality, and native parallelism**. No comfort-vs-speed compromises.

## Layout (branch `ecs`)

> ⚠️ You are on branch `ecs` — the latest active development branch. **It does not currently compile** — there are unresolved errors (`299a6b6 Blanket trait impl error fixed` was an incomplete attempt). Restoring the build is the top priority.

A workspace of crates:

- [`crates/boyko_ecs/`](crates/boyko_ecs/) — ECS core: memory, components, entities, **archetypes, queries, events**
- [`crates/boyko_macros/`](crates/boyko_macros/) — proc-macros: `#[derive(Component)]` and `#[event]`
- [`crates/boyko_utils/`](crates/boyko_utils/) — reusable collections: `BitSet`, `BitMask`, `BitSet512`, `SparseMap`, `SparseSlotMap`, `Slot`
- [`src/main.rs`](src/main.rs) — executable wrapper (currently empty; the project is library-shaped)

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

## Current branch state

| Branch | Contents | Build |
|--------|----------|-------|
| `master` | Only the memory subsystem: `Arena`, `ComponentPool<T>` (generic), `Chunk<T>` (generic), `MemFreeBlockMaster`, the basic `Entity` / `Component` types | ✅ builds |
| **`ecs`** (you are here) | Full architecture: `EcsMaster`, `ArchetypeMaster`, `Archetype`, `Query`, `Event`, `EventRegistry`, `ComponentRegistry`, type-erased `ComponentPool` + `Chunk`, `EntityMaster` with recycling, `boyko_utils` (`BitSet` / `SparseMap`) | ❌ **does not build** |

### Key architectural differences vs `master`

- **Type-erased `ComponentPool` and `Chunk`** — no longer generic over `T`. They use the `ComponentRegistry` to store the `Layout` (size + align) of each `ComponentId`. This is required for heterogeneous component storage within an archetype.
- **`Unit { ptr: *mut u8, buffer_index: usize }`** replaces the two-level `UnitId { chunk, inland }` addressing from `master`. A **direct pointer** to the component is now stored.
- **`identifiers/primitives`** — all IDs are unified as `usize`: `EntityId`, `ArchetypeId`, `ChunkId`, `ComponentId`, `Generation`, `InlandPoolId`, etc.
- **`EntityMaster`** — a real entity manager with reuse via a free list and `SparseMap<EntityInland>` for O(1) lookup.
- **`EcsMaster`** — top-level facade, returns `anyhow::Result` (questionable for a library — revisit at stabilization).
- **Global registries** (`ComponentRegistry`, `EventRegistry`) — store metadata in `static` storage. Registration happens on first access through `#[derive]`-generated code.

### Where to dig for details

- [docs/SYSTEMS.md](docs/SYSTEMS.md) — full subsystem catalog with file:line references
- [docs/FEATURE_MAP.md](docs/FEATURE_MAP.md) — "I want X → look at Y" map
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — layers, dependency graph, data flow

## Documentation — two layers

### Internal (for agents, not for publication)

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — overview of the architecture and dependencies between crates
- [docs/SYSTEMS.md](docs/SYSTEMS.md) — catalog of every subsystem with file pointers and key types
- [docs/FEATURE_MAP.md](docs/FEATURE_MAP.md) — quick lookup: "where is X implemented?"

**Every agent** must consult these files before starting work. `FEATURE_MAP.md` is the first point of contact.

### Public (mdBook + cargo doc, deployed to GitHub Pages)

- [book.toml](book.toml) — mdBook config
- [book/src/](book/src/) — sources of the public book
- [book/src/SUMMARY.md](book/src/SUMMARY.md) — table of contents (every new page is registered here)
- [.github/workflows/docs.yml](.github/workflows/docs.yml) — CI deploy to GitHub Pages

Deployment URL: `https://bluesteelll.github.io/boyko-engine/` (book) + `/api/` (rustdoc).

Local preview:
```powershell
cargo install mdbook mdbook-mermaid    # one-time
mdbook serve --open
```

The public documentation is written by the `doc-writer` agent — other agents do not edit it.

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
