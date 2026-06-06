# Benchmarking boyko-engine

This guide documents the Phase X.E benchmark methodology: how to get
**low-variance, reproducible** numbers out of criterion on a noisy Windows
development box, and how to run an honest A/B comparison.

It is infrastructure, not engine code — nothing here changes the engine's
production behavior.

## Why benches are noisy here

Most of the engine benches that exercise the ECS as a whole rebuild a world per
iteration (`EcsMaster::new`, then spawn / query / despawn). That per-iteration
construction folds two volatile costs into the timing:

- **The allocator.** The default build runs on the Windows **system heap**,
  whose latency and fragmentation behavior is the dominant run-to-run variance
  source. The documented swing on this machine is **±20-30%** — large enough to
  bury structural signals on the order of ~150 ns/entity.
- **First-touch page faults.** Fresh allocations fault their physical pages in
  lazily on first write; the cost of those faults lands inside the measured
  region whenever a bench allocates inside `b.iter`.

On top of that, the machine itself is not held steady: turbo/SpeedStep changes
the clock between samples, background processes steal cores, and codegen can
differ between two builds of the same source (multiple codegen units are
scheduled non-deterministically).

## The two structural fixes

### 1. Deterministic codegen — `[profile.bench] codegen-units = 1`

Set in the workspace root `Cargo.toml`:

```toml
[profile.bench]
codegen-units = 1
```

A single codegen unit makes two builds of the same source produce the **same**
machine code, removing codegen-layout variance from A/B deltas and hardening the
"0%-gate / byte-identical asm" methodology the perf phases rely on.

This is deliberately **not** `lto`. LTO over the Bevy dependency in
`bench_bevy_vs_boyko` would explode compile time, and LTO shifts the **mean**
rather than the **variance** — out of scope for the variance work.

> **Compile-time note.** `codegen-units = 1` in `[profile.bench]` also applies
> to dependencies. If a first bench compile of `bench-bevy-vs-boyko` (which
> pulls in Bevy) becomes painfully slow, add:
>
> ```toml
> [profile.bench.package."*"]
> codegen-units = 256
> ```
>
> That keeps **dependency** codegen parallel while the workspace crates — the
> ones whose asm we actually care about — still get `codegen-units = 1`.

### 2. Low-variance allocator — `--features bench-alloc` (opt-in)

The allocator swap is for **variance / signal extraction**, not for reporting
production absolutes, so it is **off by default**. Both bench crates
(`boyko-ecs`, `bench-bevy-vs-boyko`) expose a `bench-alloc` feature that pulls in
an optional `mimalloc` dev-dependency. Each bench binary declares:

```rust
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

`mimalloc` is far more deterministic than the Windows system heap, so enabling it
exposes structural signals the system heap masks.

- **Default `cargo bench`** keeps the system heap → **production-honest
  absolutes** (what a real binary on this OS would pay).
- **`cargo bench --features bench-alloc`** swaps in mimalloc → **low-variance
  signal** for A/B work.

Report production absolutes from the default build; use `bench-alloc` to decide
whether a change moved the needle.

## Running benches

```powershell
# Default: system heap, production-honest absolutes.
cargo bench -p boyko-ecs --bench query_iter

# Low-variance signal (mimalloc):
cargo bench -p boyko-ecs --bench query_iter --features bench-alloc

# Cross-engine comparison crate:
cargo bench -p bench-bevy-vs-boyko --bench comparison
```

For the stabilized **median-of-N** protocol, use the helper script
[`bench.ps1`](../bench.ps1) at the repo root. It runs a target N times with the
spawned cargo process pinned to High priority and to every logical core, and (if
asked) saves a criterion baseline per run for `critcmp`:

```powershell
# 3 runs (median-of-3), system heap:
./bench.ps1 -Bench query_iter

# 5 runs, mimalloc, save baselines for critcmp:
./bench.ps1 -Bench comparison -Package bench-bevy-vs-boyko -Runs 5 -BenchAlloc -Baseline mimalloc
```

`bench.ps1 -?` prints the full parameter help.

## A/B protocol

To measure whether a change helped:

```powershell
# 1. Baseline the current code.
cargo bench -p boyko-ecs --bench query_iter -- --save-baseline before

# 2. Apply your change, rebuild, then:
cargo bench -p boyko-ecs --bench query_iter -- --save-baseline after

# 3. Compare (cargo install critcmp).
critcmp before after
```

On this noisy machine a **single** before/after pair is not trustworthy.
**Repeat the pair 3-5 times and take the MEDIAN of the deltas**, discarding any
run that criterion flags with many severe outliers. `bench.ps1 -Baseline`
automates the per-run baseline naming (`<tag>_run1`, `<tag>_run2`, ...) so the
runs do not overwrite each other:

```powershell
./bench.ps1 -Bench query_iter -Runs 5 -Baseline before
# ... apply change ...
./bench.ps1 -Bench query_iter -Runs 5 -Baseline after
critcmp before_run1 after_run1   # repeat per run pair; median the deltas
```

`critcmp` install: `cargo install critcmp`.

## Manual stabilization (do this yourself — not scripted)

The script raises process priority and pins affinity, but it intentionally does
**not** touch machine-wide power/thermal state. For the most repeatable (if
slightly lower) clocks, before a measurement session:

- **Lock turbo / SpeedStep off.** Set the Windows power plan **maximum
  processor state to 99%** (this disables turbo, giving a flat base clock), or
  disable turbo in BIOS. Repeatable beats fast.
- **Close background load.** Browsers, indexers, antivirus scans, and other
  builds all steal cycles and cache.
- **Never run two bench/Miri jobs concurrently.** Concurrent jobs invalidate
  each other's numbers — this is a hard project rule.

## Read-only vs destructive benches

- **Read-only benches** (e.g. query iteration) must build the world **once,
  outside** `b.iter`, and use `black_box` only as a **sink** on the result. The
  key query benches already follow this — the timed region is the pure
  iteration hot path, with no per-iter setup or allocation.
- **Destructive benches** (spawn / despawn, which mutate or rebuild the world)
  should **amortize over a large N** and report **per-entity ratios**, not raw
  absolutes. A per-entity number is comparable across machines and across N; a
  raw "10k spawn" wall time is not.

## Deferred (out of scope for the variance work)

- **PGO** (`-Cprofile-use=...`) — shifts the mean, not the variance.
- **iai-callgrind** — gives instruction-count-exact, machine-independent
  numbers, but needs Valgrind, which has no Windows support.
- **`[profile.bench] lto`** — would help mean stability marginally but costs a
  large amount of Bevy compile time in `bench-bevy-vs-boyko`.
