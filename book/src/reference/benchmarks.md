# Benchmarks

> Where the benchmark harnesses live, which build profile each number belongs to, and where the
> results are published.

This book quotes no performance numbers. Measured results, each with the date and commit it was
taken on, are published in
[`docs/BENCHMARKS.md`](https://github.com/bluesteelll/boyko-engine/blob/master/docs/BENCHMARKS.md).
The pages here describe designs and link to that file where a number would go.

## Harnesses

Every harness is a Cargo `[[bench]]` target. The workspace has 72 of them; the larger groups are:

| Crate | Targets | What they measure |
|-------|---------|-------------------|
| [`crates/boyko_ecs/benches`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ecs/benches) | 31 | The ECS kernel: spawn, archetypes and pools, query iteration, change detection, events, hooks, states, tags, the scheduler and the app loop. |
| [`crates/boyko_physics/benches`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_physics/benches) | 17 | Broadphase, narrowphase, the coloured and parallel solve, the SIMD kernel, sleeping and soft bodies. |
| [`crates/bench_bevy_vs_boyko/benches`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/bench_bevy_vs_boyko/benches) | 9 | Head-to-head workloads against `bevy_ecs` 0.18.1, the same workload on both engines. |
| [`crates/boyko_log/benches`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_log/benches) | 8 | The cost of a disabled, gated and enabled log call, and the sink's sustained rate. |

The remaining seven are single targets in `boyko_threadpool`, `boyko_render`, `boyko_fontbake`,
`boyko_image`, `boyko_serialize`, `boyko_demo` and `reflect_fixture`.

Most targets (59 of 72) are criterion benchmarks. Eleven are hand-timed runners: a `fn main`
(`harness = false`) that times its own loop with `std::time::Instant` and prints its own report,
with no criterion baselines. They are the seven benches `boyko_log` declares, `gj1_flag_cost` in
`boyko_ecs`, and three in `boyko_physics`: `narrowphase_classes`, `omega_b_region` and
[`jolt_parity_pyramid`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_physics/benches/jolt_parity_pyramid.rs).
The last one rebuilds the pyramid scene of Jolt Physics' `PerformanceTest`, steps it from t = 0
with no warm-up and times each step, as Jolt's own test does.

The other two targets measure nothing. `instrument` in `boyko_log` is the clock helper that the
log benches include as a module; Cargo also discovers the file as a target of its own.
`reflect_optin_cost` in `reflect_fixture` is an empty placeholder (`fn main() {}`).

**Rendering has no frame-time benchmark.** The only `boyko_render` bench, `ui_pack_sort`, times the
CPU-side packing and sorting of UI instances. It renders nothing.

## Build profiles

A number is only meaningful together with the profile it was built with. The root
[`Cargo.toml`](https://github.com/bluesteelll/boyko-engine/blob/master/Cargo.toml) defines:

| Profile | Codegen | Used for |
|---------|---------|----------|
| `release` | fat LTO, default codegen units | The shipped build: `cargo build --release`. |
| `bench` | no LTO, `codegen-units = 1` | `cargo bench`. One codegen unit makes two builds of the same source lay out the same way, so A/B comparisons have low variance. It is **not** the shipped codegen. |
| `parity` | inherits `release` | The Jolt parity runner, so it compares a shipped build against Jolt's shipped build. |
| `bench-shipped` | inherits `release` | Timing a decision about the shipped codegen: `cargo bench --profile bench-shipped`. |

## Running a benchmark

```powershell
# One criterion target
cargo bench -p boyko-ecs --bench query_iter

# A median-of-N run with the process priority and affinity pinned
.\bench.ps1 -Bench comparison -Package bench-bevy-vs-boyko -Runs 5
```

Every x86_64 Windows and Linux build in this repository uses `-C target-cpu=x86-64-v3` from
`.cargo/config.toml`, so the AVX2 code paths are in every benchmark. [Installation](../getting-started/installation.md)
explains how a downstream crate keeps them.

[`docs/BENCHMARKING.md`](https://github.com/bluesteelll/boyko-engine/blob/master/docs/BENCHMARKING.md)
describes the methodology: the noise sources on a Windows desktop, the A/B protocol with criterion
baselines, and the manual stabilisation steps that
[`bench.ps1`](https://github.com/bluesteelll/boyko-engine/blob/master/bench.ps1) leaves to the
operator.

## See also

- [Design Principles](../architecture/principles.md): the performance rules the benchmarks check.
- [Parallel Scheduler](../scheduler.md) and [Physics](../simulation/physics.md): the subsystems the
  largest harnesses cover.
- [Contributing](../contributing.md): the test and lint commands.
