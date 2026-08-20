# Contributing

Thanks for your interest in Boyko Engine. This page covers the practical workflow and the standards we hold contributions to.

## Before you start

- Read the [Design Principles](architecture/principles.md). Contributions that violate them will not be merged regardless of how clean the code is.
- Skim the [Glossary](reference/glossary.md) for terminology.
- Look through open issues to avoid duplicate work.

## Setup

```powershell
git clone https://github.com/bluesteelll/boyko-engine.git
cd boyko-engine

# Build
cargo build --release

# Run the test suite
cargo test --all-targets

# Lints (must pass with zero warnings)
cargo clippy --all-targets -- -D warnings

# Format
cargo fmt
```

For documentation work:

```powershell
# Install mdBook + mermaid preprocessor once
cargo install mdbook
cargo install mdbook-mermaid

# Serve locally with live reload
mdbook serve --open
```

## Navigating the codebase

The repository ships a **graphify** knowledge graph in `graphify-out/` — a semantic map of the codebase with cross-file relationships and per-subsystem anchors. Query it **before** browsing raw source; it returns a scoped subgraph that is usually far smaller than a wide grep:

```powershell
graphify query "where is the component pool grown"   # scoped subgraph for a question
graphify explain "ComponentPool"                     # focused view of one concept
graphify path "EcsMaster" "VmReservation"            # how two things relate

# After you change code, keep the graph current (AST-only, no network call):
graphify update .
```

The internal docs are the other half of orientation: `docs/FEATURE_MAP.md` (first point of contact — "where is X"), `docs/SYSTEMS.md` (subsystem catalog with `file:line`), and `docs/ARCHITECTURE.md` (layers, dependencies, data flow). These are kept in sync with the code and are the fastest way to find a subsystem.

## Coding standards

### Performance is a feature

This is not a typical Rust crate. The engine targets ultimate performance, so:

- **No `dyn Trait`, `Box`, `Rc`, `Arc<Mutex<_>>`, `HashMap`, `Vec::new()` in hot paths.** If you need one of these, justify it in your PR description.
- **Inlining is measured, not aggressive** (see [Measured inlining](architecture/principles.md)). Use `#[inline]` on cross-crate and generic methods so their bodies stay visible to the optimizer. Reach for `#[inline(always)]` *only* when a profiler or assembly inspection shows the compiler isn't inlining and that it measurably matters — blind `#[inline(always)]` bloats the L1i cache and **lowers** performance. Use `#[cold]` / `#[inline(never)]` on error paths and rarely-taken branches to keep the hot path compact.
- **Generics + monomorphization** over runtime polymorphism.

### Unsafe code

Every `unsafe` block requires a `// SAFETY:` comment explaining the invariants:

```rust
// SAFETY: `index` is bounds-checked above. The slot was previously written
// by `add()` and the type `T` was constructed from a valid value.
unsafe { Some(&*self.data.as_ptr().add(index)) }
```

A PR with undocumented `unsafe` will not be merged.

### Naming

- `snake_case` — functions, variables, modules.
- `CamelCase` — types, traits.
- `SCREAMING_SNAKE_CASE` — constants.

### Documentation

- Every public item needs a `///` doc comment stating what it is and what guarantees it provides — this is the contract a user reads in the API reference, not a description of the implementation.
- Inline `//` comments inside a body explain **why**, not **what**: `// increment counter` above `x += 1` is noise; a comment justifying a non-obvious decision is signal.
- Use `expect("invariant: ...")` instead of `unwrap()` where a panic is by design, and `debug_assert!` for hot-path invariant checks (they vanish in release).

### Tests

- Unit tests live in `#[cfg(test)] mod tests { ... }` at the bottom of each module.
- Integration tests live in `crates/boyko_ecs/tests/`.
- Property-based tests use `proptest` and live alongside unit tests.
- `unsafe`-heavy code should be tested under [Miri](https://github.com/rust-lang/miri):

  ```powershell
  cargo +nightly miri test
  ```

- Lock-free code should be tested with [Loom](https://github.com/tokio-rs/loom).

### Benchmarks

- Use [criterion](https://github.com/bheisler/criterion.rs).
- Benchmarks live in each crate's `benches/` directory — for example `crates/boyko_ecs/benches/` for the kernel, `crates/bench_bevy_vs_boyko/benches/` for cross-engine comparisons against Bevy, plus `boyko_physics`, `boyko_serialize`, `boyko_fontbake`, and `boyko_demo`.
- `[profile.bench]` pins `codegen-units = 1` so two builds of the same source produce identical machine code — this hardens the "0%-regression / byte-identical asm" A/B methodology.
- Don't add a benchmark just for the sake of it — measure something meaningful.

### Changing the Aether DSL

If your change touches [Aether](aether/overview.md) — a new construct, a new key,
a new diagnostic — the crate's own doc carries the checklist a review runs
against. Every item on it is something that has already gone wrong, here or in
the prior art the design surveyed:

1. **Verbatim tokens, never strings.** User fragments are carried as parsed `syn` nodes and re-emitted unchanged; a `stringify!` + re-parse round trip loses spans, and everything below depends on spans.
2. **The narrowest applicable span.** The offending token — not the construct, not the block.
3. **Every diagnostic gets a `trybuild` golden.** The message is half the contract; the line and column are the other half, and the half that degrades silently.
4. **Accumulate, do not abort.** Independent constructs all report, and a broken one still emits its stub so its name resolves.
5. **Pre-check only where Aether is strictly better.** A fault rustc or a derive reports against the user's own tokens is left to them — duplicated checks drift.
6. **One table, spelling and dispatch together.** What a message advertises and what the parser accepts must be the same rows.
7. **Did-you-mean at edit distance ≤ 2**, against that same table.
8. **Emit the canonical hand-written surface.** Codegen belongs in `boyko_macros`; the expansion-volume test fails on drift in either direction.
9. **Engine paths are tokens, never dependencies** — and they must be the real nested paths, verified by a target that compiles them against the real crates.
10. **Never panic.** A panicking proc-macro erases the block from analysis; every internal failure becomes a spanned `compile_error!`.

The living version is the `aether` crate's module doc
(`crates/aether/src/lib.rs`); if the two disagree, that one is right.

## Pull request workflow

1. **Open an issue first** if your change is more than a trivial fix. Get alignment on the approach before writing code.
2. **Write the architecture plan** for non-trivial features — describe what you'll build and why before the PR.
3. **Branch from `ecs`** — the active development branch holding the full, green engine. Do **not** branch from `master`: it is a historical foundation that contains only the memory subsystem (see [Project structure](#project-structure)).
4. **Keep commits focused** — one logical change per commit.
5. **Update documentation** — both API doc comments and (if relevant) pages in `book/src/`.
6. **Pass all checks**:
   - `cargo build --release`
   - `cargo test --all-targets`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --check`
   - `mdbook build` (if docs changed)
7. **Write a clear PR description** — explain the *why*, not just the *what*. Reference the issue.

## Review process

Every non-trivial change passes through (at least) the following gates:

- **Architecture review** — does the design fit the engine's principles?
- **Code review** — does the implementation match the design? Are `unsafe` invariants sound? Are there hidden allocations?
- **Performance review** — what do the benchmarks show? Did this regress anything?

Be prepared for multiple rounds of feedback. The standards are high because the project is performance-first.

### Agent-driven workflow

Much of the project is built through a structured pipeline of specialized agents defined in `.claude/agents/`, run by an orchestrator. The roster mirrors the review gates above:

- **architect** designs a feature → **researcher** gathers practice from Bevy / flecs / EnTT / Unity DOTS → **architecture-critic** stress-tests the plan.
- **developer** implements the plan (and does *not* run the tests) → **code-reviewer** finds issues (and does *not* fix code) → **tester** runs build, unit / integration / proptest / loom, and criterion.
- **results-analyst** delivers the final verdict; **project-analyst** answers free-form questions without editing anything.
- **doc-writer** is the *only* agent that edits `book/src/`. The public mdBook is written exclusively through it — internal `docs/` are maintained separately and are not part of the published site.

Each duty is deliberately separated so that no single pass both proposes and rubber-stamps a change.

## Project structure

The workspace is a single unified engine: every system (physics, render, input, lighting, UI) is a first-class part of `boyko_ecs` — components and systems on the ECS's own storage, never a subsystem glued on the side with its own data structures.

```
boyko-engine/
├── Cargo.toml                   # workspace (18 members) + [profile.bench] + thin binary
├── src/main.rs                  # entry point (library-shaped project)
├── crates/
│   ├── boyko_ecs/               # ECS kernel: memory, components, archetypes, queries,
│   │                            #   events, scheduler, change-detection, hooks/observers,
│   │                            #   commands, states, app/plugin, serialize seam
│   ├── boyko_macros/            # #[derive(Component/Bundle/Resource/SystemSet)], #[event]
│   ├── boyko_utils/             # BitSet / BitMask / SparseMap / Slot
│   ├── boyko_threadpool/        # Chase-Lev work-stealing pool
│   │
│   ├── boyko_math/              # math primitives
│   ├── boyko_scene/             # Transform / Camera
│   ├── boyko_physics/           # in-house 3D TGS-Soft solver
│   ├── boyko_sdf_math/          # SDF math
│   ├── boyko_input/             # input
│   ├── boyko_serialize/         # codegen serialization
│   │
│   ├── boyko_rhi/               # in-house render hardware interface
│   ├── boyko_rhi_vulkan/        # raw-FFI Vulkan backend
│   ├── boyko_render/            # GPU columns, lighting, SDF render
│   ├── boyko_ui/                # ECS-native UI
│   ├── boyko_fontbake/          # MSDF atlas baker
│   ├── boyko_shaderdsl/         # shader eDSL (single-sourced host/GPU field code)
│   │
│   ├── boyko_demo/              # wgpu/egui sandbox that dogfoods the API
│   └── bench_bevy_vs_boyko/     # cross-engine comparison benches
├── book/                        # mdBook source (this site)
│   └── src/
└── docs/                        # internal documentation (agent-facing)
```

### Branches: `ecs` vs `master`

- **`ecs`** is the active development branch — the full engine, builds green. This is where all feature work goes. It carries the complete architecture: type-erased `ComponentPool` / archetypes, the typed `Query<D, F>` DSL, `Commands` / `EntityCommands`, events, resources, lifecycle hooks and observers, required components, entity cloning, generic relations (`ChildOf` / `Children`), states, schedule ordering and sets, run conditions, the `App` + `Plugin` facade, fixed timestep, multi-world, and dense (non-fragmenting) components — plus the in-house RHI + raw-FFI Vulkan render path, SDF (sphere-trace + brick atlas + shader eDSL), clustered lighting, the in-house physics solver, ECS-native UI, input, and codegen serialization.
- **`master`** is a historical foundation containing the memory subsystem only (the generic `ComponentPool<T>` / `Chunk<T>` with two-level addressing). It is **not** where new work goes.

## Reporting issues

When filing a bug:

- Provide a minimal reproducer.
- Include `cargo --version`, `rustc --version`, and your OS.
- For performance issues, include the benchmark you ran and the numbers you got.

When filing a feature request:

- Describe the use case, not just the desired API.
- If you've looked at how Bevy/flecs/EnTT handle this, mention it.

## Communication

- **Issues** — bugs, feature requests, design discussions.
- **Discussions** — broader questions, ideas, polls (if enabled on the repo).

## License

By contributing, you agree your contributions will be licensed under the same terms as the project (see the repository for details).

---

Thank you for helping push Rust ECS forward.
