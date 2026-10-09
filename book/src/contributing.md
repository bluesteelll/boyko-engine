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

# Run the test suite: every member, and every target even after one fails
cargo test --workspace --all-targets --no-fail-fast

# Lints (must pass with zero warnings)
cargo clippy --workspace --all-targets -- -D warnings
```

Both flags matter. Without `--workspace`, a command run from the repository root checks
only part of the workspace and still reports success. Without `--no-fail-fast`, the first
failing test target stops the run and hides every target after it.

**Toolchain and platforms.** The engine is Rust edition 2024 on the stable channel
([`rust-toolchain.toml`](https://github.com/bluesteelll/boyko-engine/blob/master/rust-toolchain.toml)).
It targets x86-64 with AVX2 as the baseline (`.cargo/config.toml` builds with
`-C target-cpu=x86-64-v3`) on Windows and Linux. The development host is Windows with
the MSVC toolchain: `stable-x86_64-pc-windows-msvc` for builds and
`nightly-x86_64-pc-windows-msvc` for Miri.

**Formatting.** Format the code you add with rustfmt; the tree is not yet
rustfmt-normalised and CI does not check formatting.

For documentation work:

```powershell
# Install mdBook + mermaid preprocessor once
cargo install mdbook
cargo install mdbook-mermaid

# Serve locally with live reload
mdbook serve --open
```

## Navigating the codebase

Start with the internal docs. `docs/FEATURE_MAP.md` is the first point of contact: it answers "where is X?". From there, `docs/SYSTEMS.md` catalogs each subsystem with `file:line` references, and `docs/ARCHITECTURE.md` covers layers, crate dependencies, and data flow. They are the fastest way to find a subsystem. If an anchor and the source disagree, the source is right.

Once you know where to look, let the tools answer questions about the code itself. [rust-analyzer](https://rust-analyzer.github.io/) is the ground truth for a symbol: *Go to Definition*, *Find References*, and *Go to Implementation* follow a type or a call across crates. Hovering a type also shows its computed size and alignment. For literal or pattern-shaped searches — a string, an attribute, every `// SAFETY:` comment — use [ripgrep](https://github.com/BurntSushi/ripgrep):

```powershell
rg -n "// SAFETY:" crates/boyko_ecs/src   # every SAFETY justification in the ECS kernel
```

## Coding standards

### Performance is a feature

This is not a typical Rust crate. The engine targets ultimate performance, so:

- **No `dyn Trait`, `Box`, `Rc`, `Arc<Mutex<_>>`, `HashMap`, `Vec::new()` in hot paths.** If you need one of these, justify it in your PR description.
- **Inlining is measured, not aggressive** (see [Measured inlining](architecture/principles.md)). Use `#[inline]` on cross-crate and generic methods so their bodies stay visible to the optimizer. Reach for `#[inline(always)]` *only* when a profiler or assembly inspection shows the compiler isn't inlining and that it measurably matters — blind `#[inline(always)]` bloats the L1i cache and **lowers** performance. Use `#[cold]` / `#[inline(never)]` on error paths and rarely-taken branches to keep the hot path compact.
- **Generics + monomorphization** over runtime polymorphism.

### Unsafe code

Every `unsafe` block requires a `// SAFETY:` comment explaining the invariants:

```rust,ignore
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
- Integration tests live in each crate's own `tests/` directory. Repository-wide census
  tests live in the root `tests/` directory (for example `tests/ignore_reasons_census.rs`).
- Property-based tests use `proptest` and live alongside unit tests.

### Testing

- **Ignored tests carry a class.** `cargo test` runs none of the `#[ignore]`d tests. Each
  one's reason starts with a class from a closed list —
  `#[ignore = "<class>: <prose>"]` with `class` one of `gpu`, `gpu-windowed`, `gpu-cap`,
  `feature`, `solo`, `slow`, `miri-slow`, `miri-unsupported`, `generator`, `deferred`,
  `flaky` — and the census test `tests/ignore_reasons_census.rs` fails the build on an
  ignore without one. A composite spells two classes (`feature+gpu`,
  `gpu-windowed+gpu-cap`). The class says which leg runs the test:
  - `gpu`, `gpu-windowed` and `gpu-cap` need a Vulkan device (`gpu-windowed` also a
    desktop window; `gpu-cap` a capability such as ray queries). Run them one test binary
    at a time with `-- --ignored --test-threads=1`; each file's header names the
    environment variables it reads.
  - `feature` tests compile only with a named Cargo feature (for example `--features hwrt`).
  - `solo` and `slow` are device-free legs: run them with `-- --ignored`, `solo` with
    `--test-threads=1`.
  - `generator`, `deferred` and `flaky` belong to no leg; do not sweep them into a run.
  - A leg that prints `running 0 tests` did not run anything — it is not a pass.
- **Miri.** `unsafe`-heavy code is tested under [Miri](https://github.com/rust-lang/miri),
  on the nightly toolchain:

  ```powershell
  cargo +nightly-x86_64-pc-windows-msvc miri test -p <crate>
  ```

  `.cargo/config.toml` turns on Tree Borrows. Tests that still run natively but that Miri
  would need hours for carry `miri-slow`, and tests Miri cannot execute at all (a child
  process, a custom global allocator, a deliberate leak) carry `miri-unsupported`.
- **Loom.** Lock-free protocols are model-checked with
  [Loom](https://github.com/tokio-rs/loom) by the `loom_*.rs` tests in
  `crates/boyko_threadpool/tests` and `crates/boyko_ecs/tests`. They compile only under
  `--cfg loom`; the `loom` job in `.github/workflows/ci.yml` shows the exact flags.
- **Golden images.** `scripts\golden.ps1` renders frames and checks them against the
  SHA-256 pins in `goldens/PINS.toml`. The pins are byte-identity hashes blessed on one
  NVIDIA RTX 3060, so they hold for that device only; on other hardware a mismatch is not
  by itself a regression.

### Benchmarks

- Use [criterion](https://github.com/bheisler/criterion.rs).
- Benchmarks live in each crate's `benches/` directory: `boyko_ecs` (the kernel), `bench_bevy_vs_boyko` (cross-engine comparisons against `bevy_ecs`), `boyko_physics`, `boyko_render`, `boyko_threadpool`, `boyko_serialize`, `boyko_fontbake`, `boyko_image`, `boyko_log`, `boyko_demo`, and the `reflect_fixture` test fixture.
- `[profile.bench]` pins `codegen-units = 1` and turns LTO off, so two builds of the same source produce identical machine code — this hardens the "0%-regression / byte-identical asm" A/B methodology. A number taken under `bench` therefore does not describe the shipped `release` build; name the profile with every result.
- Don't add a benchmark just for the sake of it — measure something meaningful.
- Published results live on the [Benchmarks](reference/benchmarks.md) page.

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
3. **Branch from `master`.** It holds the whole engine.
4. **Keep commits focused** — one logical change per commit.
5. **Update documentation** — both API doc comments and (if relevant) pages in `book/src/`.
6. **Pass all checks**:
   - `cargo build --release`
   - `cargo test --workspace --all-targets --no-fail-fast`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - the ignored-test legs your change touches (see [Testing](#testing))
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

```text
boyko-engine/
├── Cargo.toml                   # workspace (33 members) + profiles + the root package
├── src/main.rs                  # placeholder binary (library-shaped project)
├── tests/                       # repository-wide census tests
├── crates/
│   ├── boyko_ecs/               # ECS kernel: storage, archetypes, queries, systems,
│   │                            #   scheduler, commands, events, change detection,
│   │                            #   hooks/observers, relations, states, assets, App/Plugin
│   ├── boyko_memory/            # virtual-memory reserve/commit primitive and column
│   ├── boyko_macros/            # derives and macros: Component, Bundle, Resource,
│   │                            #   SystemSet, #[event], ui!, state_chart!, ...
│   ├── boyko_utils/             # BitSet / BitMask / SparseMap / Slot
│   ├── boyko_threadpool/        # Chase-Lev work-stealing pool
│   ├── boyko_diag/              # diagnostics substrate: clock, lane topology
│   ├── boyko_log/               # in-house structured logging, boyko-Cnnnn codes
│   │
│   ├── boyko_math/              # SIMD-aligned, deterministic POD math
│   ├── boyko_scene/             # Transform / Camera / visibility
│   ├── boyko_physics/           # in-house 3D TGS-Soft rigid and XPBD soft-body physics
│   ├── boyko_sdf_math/          # analytic SDF field math (render + physics)
│   ├── boyko_input/             # rebindable action mapping
│   ├── boyko_serialize/         # binary world save / load
│   ├── boyko_reflect/           # editor-build reflection (experimental)
│   │
│   ├── boyko_rhi/               # in-house render hardware interface
│   ├── boyko_rhi_vulkan/        # raw-FFI Vulkan backend, framegraph, window
│   ├── boyko_render/            # ECS-to-RHI bridge: GPU columns, render paths,
│   │                            #   lighting, shadows, GI, AA, particles, loaders
│   ├── boyko_shaderdsl/         # shader eDSL (single-sourced host/GPU shader math)
│   ├── boyko_image/             # in-house PNG / zlib / DEFLATE decoder
│   ├── boyko_fontbake/          # MTSDF font atlas baker
│   ├── boyko_ui/                # ECS-native UI
│   ├── boyko_app/               # host layer: OS loop, device boot, windowed runner,
│   │                            #   EnginePlugins, the examples
│   │
│   ├── aether_lang/             # Aether DSL: parser and expander
│   ├── aether/                  # the aether! macro
│   ├── aether_tests/            # Aether integration and trybuild tests
│   │
│   ├── boyko_demo/              # eframe/wgpu sandbox that dogfoods the ECS API
│   ├── bench_bevy_vs_boyko/     # cross-engine comparison benches
│   ├── boyko_symcensus/         # dev-only symbol census of post-LTO builds
│   ├── reflect_fixture/         # test fixtures for the reflection and
│   ├── reflect_dogfood/         #   profiling gates
│   ├── profile_fixture/
│   └── profile_fixture_log/
├── tools/
│   └── prof_decode/             # prints a profiling telemetry stream as text
├── book/                        # mdBook source (this site)
│   └── src/
└── docs/                        # internal documentation (agent-facing)
```

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

Licensed under the Apache License, Version 2.0 ([LICENSE](https://github.com/bluesteelll/boyko-engine/blob/master/LICENSE)). Some files are third-party and remain under their own licenses; they are listed in [NOTICE](https://github.com/bluesteelll/boyko-engine/blob/master/NOTICE) and [THIRD-PARTY-NOTICES.md](https://github.com/bluesteelll/boyko-engine/blob/master/THIRD-PARTY-NOTICES.md).

By contributing, you agree that your contributions are licensed under the same terms.

---

Thank you for helping push Rust ECS forward.
