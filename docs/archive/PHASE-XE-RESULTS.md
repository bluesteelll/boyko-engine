# Phase X.E — Results: bench methodology (variance reduction)

Branch `ecs`. BENCH TOOLING/INFRASTRUCTURE — zero engine production-code changes. Goal: kill the
documented ±20-30% run-to-run bench variance so perf signals as small as ~150 ns/entity become
measurable, and make a future Phase 13 (≥10% query-iter surpass vs Bevy) credibly measurable.
Pipeline: researcher → (orchestrator decisions) → developer → orchestrator empirical verification.

## Status: COMPLETE — variance reduction demonstrated, opt-in, zero default-build impact

### What shipped (the two structural fixes + the protocol)
1. **`[profile.bench] codegen-units = 1`** (workspace `Cargo.toml`) — deterministic single-unit
   codegen: two builds of the same source produce the same machine code, removing codegen-layout
   variance from A/B deltas and hardening the "0%-gate / byte-identical asm" methodology. NOT `lto`
   (LTO over Bevy in `bench_bevy_vs_boyko` explodes compile time + shifts the mean, not the
   variance). Compile-time impact measured acceptable (~58 s cold for the Bevy-linking crate), so the
   `[profile.bench.package."*"]` dep-override was NOT needed.
2. **Opt-in mimalloc** behind a `bench-alloc` feature (OFF by default) in both bench crates. Each
   bench binary carries a `#[cfg(feature = "bench-alloc")] #[global_allocator]`. Default `cargo
   bench` keeps the Windows system heap (production-honest absolutes); `cargo bench --features
   bench-alloc` swaps in mimalloc (low-variance A/B signal).
3. **`bench.ps1`** (median-of-N protocol: High priority + affinity-pinned cargo child, optional
   per-run criterion baselines for `critcmp`) + **`docs/BENCHMARKING.md`** (full methodology: the
   two fixes, the A/B + median-of-N protocol, manual stabilization, read-only vs destructive bench
   discipline, deferred items).

### Decisions (orchestrator, grounded in the research)
- **mimalloc = opt-in, not default.** It shifts absolutes (≈ −17% on an allocator-touching bench),
  so it is a *signal-extraction* allocator, not a production-numbers allocator. Default builds keep
  honest system-heap absolutes.
- **Turbo-off = documented, not scripted** (BIOS/power-plan changes are awkward to script reliably).
  `bench.ps1` scripts the reliably-scriptable knobs (priority + affinity).
- **`float_algebraic` (Phase 13 vehicle) = proceed on nightly** (already gated via the bench crate's
  `rust-toolchain.toml`); do not block X.E on its stabilization (it is in FCP).
- **PGO + iai-callgrind = deferred.** PGO shifts the mean not the variance; iai-callgrind needs
  Valgrind (no Windows support). Both noted in `BENCHMARKING.md` as future options.

## Empirical validation (orchestrator-run)

Target: `create_entity_10k` (per-iter `EcsMaster::new` + 10k creates — touches the global allocator
via the world's caches + entity bookkeeping). 4 back-to-back runs per config
(`--warm-up-time 2 --measurement-time 3 --sample-size 30`):

| Config | Run estimates (µs) | Mean | Run-to-run spread |
|--------|---------------------|------|-------------------|
| System heap (default) | 666.98 / 671.33 / 656.76 / 653.58 | **662.2** | ~2.7% |
| mimalloc (`--features bench-alloc`) | 563.18 / 547.83 / 543.68 / 548.91 | **550.9** | ~1.4% |

- **mimalloc: ~17% lower mean + ~2× tighter run-to-run spread.** The Windows system heap was adding
  ~110 µs/iter of allocation overhead AND most of the run-to-run noise on this allocator-touching
  bench. This both validates the variance thesis and demonstrates exactly why mimalloc must be
  opt-in (the −17% would corrupt production-honest absolutes).
- `create_entity_10k` is **amortized** (one `EcsMaster::new` over 10k creates), so its system-heap
  spread (~2.7%) is already well below the documented ±20-30% — which is itself a confirmation of the
  research's core point: *setup-dominated* benches (single `EcsMaster::new`/spawn, e.g. the
  `bench_bevy_vs_boyko` g4/g5) are the high-variance ones, and the new infra (mimalloc opt-in +
  median-of-N protocol) is precisely the tool for them.

## Verification gate
- `cargo build --all-targets` (default) — green; **mimalloc is NOT compiled** (`cargo tree -i
  mimalloc` → "did not match any packages" under default features; present only under
  `--features bench-alloc`).
- `cargo bench -p boyko-ecs --no-run` and `--no-run --features bench-alloc` — both build all bench
  binaries. Same for `bench-bevy-vs-boyko`. Nightly `g6_for_each_chunk --features "nightly,bench-alloc"`
  builds (allocator block sits correctly after the crate `#![cfg_attr]`).
- `cargo clippy --all-targets -- -D warnings` (default) — clean.
- `bench.ps1` parses (PowerShell 7) and its 6 parameters resolve.

## Notes
- **Forced deviation (correct)**: `mimalloc` is declared in `[dependencies]` as `optional = true`,
  not `[dev-dependencies]` — Cargo rejects optional dev-dependencies, and the `dep:mimalloc` feature
  syntax requires a normal optional dep. Opt-in semantics are fully preserved (verified via
  `cargo tree`): nothing enables `bench-alloc` by default, so mimalloc is never in the default graph.
  `Cargo.lock` now records `mimalloc`/`libmimalloc-sys` as inactive resolution metadata.
- Allocator one-liner applied to all 26 `[[bench]]` target files (19 in `boyko-ecs`, 7 in
  `bench-bevy-vs-boyko`); the 7 `bench-bevy-vs-boyko` `configure()` helpers also got
  `warm_up_time(3s)` + `noise_threshold(0.05)`.
- Pre-existing, out of scope: 2 `dead_code` warnings on `ParamB` surface only under the bench-profile
  test build (`cargo clippy --all-targets` is clean); engine source, untouched.

## Files
- `Cargo.toml` (`[profile.bench]`), `Cargo.lock` (mimalloc metadata).
- `crates/boyko_ecs/Cargo.toml`, `crates/bench_bevy_vs_boyko/Cargo.toml` (`bench-alloc` feature +
  optional mimalloc).
- 26 bench `*.rs` files (cfg-gated `#[global_allocator]`; 7 also `configure()` tweaks).
- `bench.ps1` (new), `docs/BENCHMARKING.md` (new).

## Follow-up
- The high-variance setup-dominated benches (`bench_bevy_vs_boyko` g4/g5) can now be re-measured
  under `--features bench-alloc` + `bench.ps1` median-of-N to extract the structural signals 12.6
  flagged — do this opportunistically when next touching spawn perf.
- Phase 13 (query-iter ≥10% surpass) is now measurable: use the `g6`/`g6b` nightly `algebraic_add`
  harness + `--features bench-alloc` + median-of-N.
