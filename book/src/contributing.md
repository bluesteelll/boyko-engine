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

## Coding standards

### Performance is a feature

This is not a typical Rust crate. The engine targets ultimate performance, so:

- **No `dyn Trait`, `Box`, `Rc`, `Arc<Mutex<_>>`, `HashMap`, `Vec::new()` in hot paths.** If you need one of these, justify it in your PR description.
- **`#[inline]` and `#[inline(always)]`** are used aggressively on accessors and trampoline functions.
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

- Every public item needs a `///` doc comment.
- Doc comments explain **what** the item is and **what guarantees** it provides — not how it's implemented.
- Implementation details go in `//` comments inside the body, and only when "why" is non-obvious.

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
- Benchmarks live in `crates/boyko_ecs/benches/`.
- Don't add a benchmark just for the sake of it — measure something meaningful.

## Pull request workflow

1. **Open an issue first** if your change is more than a trivial fix. Get alignment on the approach before writing code.
2. **Write the architecture plan** for non-trivial features — describe what you'll build and why before the PR.
3. **Branch from `master`**.
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

PRs go through (at least) the following:

- **Architecture review** — does the design fit the engine's principles?
- **Code review** — does the implementation match the design? Are `unsafe` invariants sound? Are there hidden allocations?
- **Performance review** — what do the benchmarks show? Did this regress anything?

Be prepared for multiple rounds of feedback. The standards are high because the project is performance-first.

## Project structure

```
boyko-engine/
├── Cargo.toml              # workspace
├── src/main.rs             # binary entry point
├── crates/
│   ├── boyko_ecs/          # ECS core
│   ├── boyko_macros/       # proc-macros
│   └── boyko_utils/        # bitsets (on `ecs` branch)
├── book/                   # mdBook source
│   └── src/
└── docs/                   # internal documentation
```

The `master` branch holds the stable foundation (currently the memory subsystem only). The `ecs` branch contains in-progress work on archetypes, queries, and events.

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
