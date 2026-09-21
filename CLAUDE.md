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
cargo +nightly-x86_64-pc-windows-msvc miri test                  # UB detector (if nightly is installed)
```

**On the workstation, a gate run sets `TMP`/`TEMP` to `D:/wt/_targets/tmp` first** (RK-18 of the
unification plan), and the directory must exist before cargo starts — `link.exe` writes its own
temporaries there, and a missing path is `LINK : fatal error LNK1104: cannot open file
'…\lnk{…}.tmp'` (MEASURED 2026-09-21 on a one-file crate under `stable-x86_64-pc-windows-msvc`):

```powershell
$env:TMP = 'D:/wt/_targets/tmp'; $env:TEMP = $env:TMP; New-Item -ItemType Directory -Force $env:TMP | Out-Null
```
```bash
mkdir -p D:/wt/_targets/tmp && export TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp
```

The reason: `crates/profile_fixture/tests/profile_axis_census.rs` roots its six fat-LTO builds
(three fixture binaries × two profiles) in two target trees keyed by profile and host (`:226`), and
its refusal probe's `cargo check` tree (`:760`), at `std::env::temp_dir()`, which on Windows is
`%TMP%`/`%TEMP%`, i.e. drive C:. RK-11 puts every build product under `D:/wt/_targets`; without this
line the census is the one exception, and the other 47 files that write artifacts through
`temp_dir()` follow it. Measured 2026-09-21 on C:: four census trees (the two host-keyed ones plus
the two left from before `27ac8904` keyed them), 11–17 MB each, and the refusal tree at 803 MB
(675 MB of it `incremental/`, from runs without `CARGO_INCREMENTAL=0`) — the rule is one drive for
every build product, whatever the size.

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

### The feature axis — `--all-targets` is not `--all-features`

**None of the four commands above compiles a single `#[cfg(feature = "…")]` item.** `--all-targets`
widens the TARGET set (lib, bins, tests, benches, examples) and leaves the cfg set at the default
one; the two flags are unrelated, and the names are close enough that this tree ran on the confusion
for 32 days. The standing gate is therefore not *weak on* feature-gated code, it is **blind** to it:
that code is never handed to rustc, and rustc cannot go red over a span it never sees.

**MEASURED 2026-09-18, on one tree, one minute apart.** With two shipped lines spelling
`W2102.number()` / `W2106.number()` — a `u16` where `warn!` takes a typed `WarnCode` —
`cargo check -p boyko_rhi_vulkan --features hwrt --lib` failed with **10 errors**, while
`cargo check -p boyko_rhi_vulkan --lib` over the *identical* source finished green in **2.49 s**.
`718a5129` (2026-08-17) introduced that type and converted every site rustc showed it, including
**both** non-gated `W2102.number()` sites in the same three-function group of `device.rs`, and left
the `#[cfg(feature = "hwrt")]` sibling sitting between them. The feature had not compiled for
**~248 commits / 32 days**, and what finally compiled it was a person running the GPU leg by hand.

⚠️ **The one-flag answer does not exist in this tree, and that is by design rather than neglect.**
`--all-features` turns on `boyko-threadpool/tb-neg-m2w`, whose crate root is a `compile_error!`
outside Miri (the KE16 M2w deliberate-UB negative control), and three `nightly` features that are
E0554 on stable. MEASURED 2026-09-18: `cargo check --workspace --all-targets --all-features` dies on
`boyko_shaderdsl`'s E0554 before reaching one engine crate. **Per-feature legs are the only form that
can work here**, and CI's `feature-legs` job runs them — deriving its leg set as *every feature
`cargo metadata` reports, minus a refusal table*, so a feature added tomorrow is covered by default
and has to be argued out in writing.

**The enumeration, because a check that covers one feature while implying it covers all is the
failure this gate exists to stop.** 23 non-default features across 11 packages, measured 2026-09-18:

- **Covered — 15 legs, run on `ubuntu-latest`:** `boyko-diag/section-gate`;
  `boyko-ecs/{bench-alloc, big_query_table, profiling-analysis}`; `boyko-log/test-probe`;
  `boyko-physics/bench-alloc`; `boyko-render/{hwrt, profiling-census, test-readback}` (at
  `--lib --tests` — see the scope table note below); `boyko-threadpool/scheduler-trace`;
  `boyko_rhi_vulkan/{goldens, hwrt, profiling-census, spec_constant_smoke}`; `boyko_shaderdsl/emit`.
  `boyko-render/test-readback` is a leg but **adds no coverage of its own**: the crate's
  self-referential dev-dependency turns that feature on in every `--tests` build. VERIFIED
  2026-09-18 by running the job's own loop from a Windows host with `--target x86_64-unknown-linux-gnu`
  appended: 13 legs green with 0 warnings. The two `bench-alloc` legs cannot be cross-checked that
  way, because `libmimalloc-sys`'s build script needs an `x86_64-linux-gnu-gcc`, which the runner has
  and a Windows box does not. Both are green natively on msvc.
- **NOT covered — 8, each with its reason in `ci.yml`:** `boyko-app/{hwrt, profiling-alloc,
  profiling-census}` — *that crate's **lib** does not build off Windows*, so those legs belong on a
  developer box, not on the runner; `bench-bevy-vs-boyko/{bench-alloc, nightly}` — the bevy tree,
  excluded from every job but `bench-compile`; `boyko-threadpool/tb-neg-m2w` — `compile_error!`
  outside Miri, by design, gated instead by `scripts/tb_neg_gate.{ps1,sh}`; `boyko_sdf_math/nightly`
  and `boyko_shaderdsl/nightly` — **KNOWN-RED, an open defect** (below).
- **What no leg covers at all:** clippy lints on gated code. The legs are `cargo check`, so rustc
  warnings fail them (CI's `RUSTFLAGS` carries `-D warnings`), but no clippy lint has ever run over
  a `#[cfg(feature = …)]` item in this tree. Narrowed here, not closed.

⚠️ **Three of those 15 legs exist only because a refusal row was re-measured and withdrawn, and a
refusal that overstates its reason is the same defect as a coverage claim that overstates its
coverage.** `boyko-render` was excluded as a crate that "does not compile for a non-Windows target
at all", on a measurement of **2 errors**. RE-MEASURED 2026-09-18 on the same triple, those 2 were
**one** `error[E0601]` — `main` function not found in `examples/orbit_cube_window.rs`, which is
`#![cfg(windows)]` as a whole crate by construction — plus cargo's own summary line (`--keep-going`
confirms no second failing target hides behind it). The **lib compiles**:
`cargo check -p boyko-render --features hwrt --lib --tests --target x86_64-unknown-linux-gnu` exits
0 in 10.8 s from a clean `cargo clean -p boyko-render`. The false row had excluded three legs and
left **34 `#[cfg(feature = "hwrt")]` sites** under `crates/boyko_render/src` compiled by nothing —
the exact class this job was created to close, one crate over from where it was found. `ci.yml`
therefore carries a **second table beside the refusals**: a leg whose *lib* builds on the runner but
whose *example* does not gets a narrower TARGET SET, never an exclusion. An exclusion is only for a
crate whose lib itself cannot compile there — `boyko-app`, measured below. Both tables fail the
derive step when a row outlives its subject, and the leg floor is pinned at 15.

Locally a feature is checked per crate — this is the whole recipe:

```powershell
cargo check -p boyko_rhi_vulkan --features hwrt --all-targets    # the leg that was red for 32 days
cargo check -p boyko-app        --features hwrt --all-targets    # Windows-only; CI cannot run it
cargo check -p boyko-render     --features hwrt --all-targets    # the example CI's scoped leg drops
```

VERIFIED 2026-09-18 on `stable-x86_64-pc-windows-msvc`: all three lines are green (0 errors,
0 warnings), and so are `boyko-app`'s `profiling-alloc` and `profiling-census` at `--all-targets`.
For those three `boyko-app` features and for `orbit_cube_window`, this by-hand recipe is the **only**
compile they get. No CI job runs it, so it is a recipe, not coverage.

⚠️ **Two features are RED right now, and the job refuses them by name rather than hiding them.**
`boyko_shaderdsl/nightly` (and `boyko_sdf_math/nightly`, which forwards to it) makes the crate
`no_std` while `src/ssao.rs` uses `format!`/`String`, and calls `core::intrinsics::{sinf32, cosf32}`,
which current nightly no longer has: RE-MEASURED 2026-09-18, **10 errors in the lib on
`nightly-x86_64-pc-windows-msvc`** and **9 on `stable-x86_64-pc-windows-msvc`** (E0554 among them),
identical under `--lib` and `--all-targets` — so it is broken on *both* channels, not merely
channel-gated. Covering it would make the job permanently red, and a permanently red job is one
nobody reads; the refusal row carries the date and the error instead, which is the form this
repository already uses for a known red. Two details the first pass got wrong and the re-measure
fixed, because a refusal's numbers are a claim like any other: the count was written as **11**,
while the lib target gives 10 on nightly and 9 on stable at either scope (`--all-targets`
additionally fails the lib-test unit, 8 on stable / 9 on nightly, RE-MEASURED 2026-09-18 with
`--keep-going`), and `boyko_sdf_math/nightly` dies **inside the dependency** — that crate is never
compiled at all, so the count was never its own.

⚠️ **The identical blind spot exists on the PLATFORM cfg, and for `boyko-app` it is NOT closed.**
One `cargo` invocation compiles one target triple, so a `#[cfg(not(windows))]` item is exactly as
invisible as a feature-gated one — and it has already produced the same defect:
`crates/boyko_app/src/diag.rs:184` still spells `codes::E3004.number()` inside a
`#[cfg(not(windows))]` reporter. It landed 2026-08-13 (`b809041e`), **four days before** the sweep
that fixed its siblings (`718a5129`), and that sweep — which worked off rustc's own E0308 spans on a
Windows box — could not be shown it. RE-MEASURED 2026-09-18 with `--keep-going`:
`cargo check -p boyko-app --lib --target x86_64-unknown-linux-gnu` fails on DEFAULT features with
five printed diagnostics — 2 × E0432 (`crate::gpu_scene` is a `#[cfg(windows)]` module, imported
unconditionally at `runner.rs:77` and `particle_readback.rs:34`) and 3 × E0308, all at
`diag.rs:184` — which rustc's summary line counts as **"7 previous errors"**. The failure is in the
**lib**, so the ubuntu runner cannot build `boyko-app` at all, and **no CI leg covers its cfg arms
today**.

**`boyko-render` is NOT in that position, and saying it was is the error the scope table above
corrects.** Its lib builds on the runner, and the three scoped feature legs compile it,
`cfg(not(windows))` arms included. The one thing the runner cannot build is
`examples/orbit_cube_window.rs`, which is `#![cfg(windows)]` and therefore has no `main` there; no
CI job compiles that example.

⚠️ **One consequence reaches past the feature axis.** The `check`, `test`, `profile-legs`,
`clippy`, `force-alloc-panic` and `bench-compile` jobs all run `--workspace` on `ubuntu-latest`, and
their target sets include `boyko-app`'s lib (all six) and that example (the five `--all-targets`
jobs). A failing unit makes cargo exit non-zero, so **none of those jobs can pass on the runner as
the tree stands.**
This is inferred from the two per-crate measurements plus the command lines. `bench-compile`'s
default target selection is the non-obvious one, and it was checked:
`cargo +nightly bench --no-run --workspace --exclude boyko_demo --target x86_64-unknown-linux-gnu
-Z unstable-options --unit-graph` lists `boyko_app`'s lib and no example. The workspace-wide
commands themselves were not run for the linux triple here, and no CI run history was consulted.
The repair belongs in those two crates, not in the gate.

### The ignored suite — legs by what the machine has

The four commands above run **none** of the `#[ignore]`d tests — **333 sites (183 unconditional +
150 `#[cfg_attr(<cfg>, ignore = …)]`), measured 2026-09-21 on `merge/ke16-into-ecsnative` @
`ff64c6be`, the union of the light-table and parallel-narrowphase lanes.** The last move,
328 → 333, is that union and nothing else: five plain sites, none removed, none `cfg_attr`,
attributed by `git diff` against each merge's second parent — the light-table lane's **+4
`gpu-windowed:`** in `boyko_app` (`sdf_marcher_sun.rs` ×3 and `unwritten_shadow_map_gate.rs`'s
`unslotted_punctual_lights_never_sample_the_atlas`) and the narrowphase lane's **+1 plain `slow:`**
in `boyko_physics/tests/narrowphase_parallel_equivalence.rs`
(`jolt_pyramid_parallel_narrowphase_is_bit_identical`, L5 C3). The light-table lane's own "324"
was 320 + 4 on its branch point, which predates the L2 calibration's +7 and the simd_solve +1; an
independent enumeration reproduces 333 / 183 / 150 across 10 crates and 1,642 `.rs` files walked.
The move before it, 321 → 328, is the parallel-narrowphase lane's L2 calibration: seven
`cfg_attr(miri, "miri-slow: …")` sites in `broadphase_select_p3.rs`: `GRID_LO`/`GRID_HI` moved from
96/192 to 2,700/3,000, so the six existing tests, which size their scenes from the band, now step
2,692–3,016 spheres, and the new G-L2-1 steps the 1,241-body Jolt pyramid. The move before it,
320 → 321, is the colored-solver-default lane's `simd_solve_on_off_bit_identical`, a
`cfg_attr(any(miri, debug_assertions), "slow: …")` site. The move before it, 310 → 320, is the
boot-validation lane's ten `gpu-windowed:` device tests: Gate A's driver and six workers in
`boot_validation_clean.rs`, and the shadow gate's two drivers and one worker in
`unwritten_shadow_map_gate.rs`. The move before that, 311 → 310, is the A7 lane's four
`deferred:` red-first tests: A7-R0 is no longer
ignored, A7-R1 and A7-R2 became `cfg_attr(any(miri, debug_assertions), "slow: …")`, and G8 became
`cfg_attr(miri, "miri-slow: …")`. The move before it, from 2026-09-17's 280, is two things and not
one: the A6 lane's three new test files brought **32**
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
**159 of the 183 plain sites** (`boyko_app` 102, `boyko_rhi_vulkan` 51, `boyko_render` 6, measured
2026-09-21 on the union; 155 of 178 on 2026-09-19, the four since being the light-table lane's
`gpu-windowed:` tests in `boyko_app`; 145 of 172 on 2026-09-17, the ten between being the
boot-validation lane's), which is the population this leg is drawn from, not the leg itself. The
other 24 plain sites are `boyko_ecs` 10, `boyko_serialize` 5, `boyko_physics` 3, `boyko_sdf_math` 2,
`boyko_ui` 2, `boyko_log` 1, `boyko_threadpool` 1. **Five** of the 159 need a non-default cargo
feature and are not even *compiled* otherwise: `--features hwrt` ×4
(`boyko_rhi_vulkan/tests/hwrt_blas_smoke.rs` 3, and `boyko_app/tests/asset_streaming_f7_rt_cap_headless.rs`
1 — a `#![cfg(feature = "hwrt")]` file present since `b8d8b162`, 2026-07-10, so the "×3" this
paragraph carried until 2026-09-21 was an undercount from the day it was written, not a new site)
and `--features spec_constant_smoke` ×1. **121 of the 183** plain sites sit in files under
`#![cfg(windows)]` and vanish on Linux (the earlier "most of the rest" was never counted). There is
no single command — each binary has its own env-var protocol in its module header
(`BOYKO_DISABLE_VALIDATION`, `BOYKO_HZB_DUMP`, `BOYKO_WINDOW_FRAMES`, …).

**Leg: Miri.** `cargo +nightly-x86_64-pc-windows-msvc miri test` already carries **149** of the ignores (measured
2026-09-19 on the parallel-narrowphase lane after its L2 calibration and re-measured unchanged
2026-09-21 on the union: neither merged lane added a `cfg_attr` site, so the 150 did not move and
the per-cfg split 142 / 6 / 2 is the same) — the **148** `cfg_attr` sites
whose cfg is `miri` (142) or `any(miri, debug_assertions)` (6), plus `miri_fixed_loop`'s one plain
ignore. The `miri` sites run
*natively* in both profiles and are skipped only under Miri; the `any(miri, debug_assertions)`
sites run natively in RELEASE only — their leg is the physics release run below; there are six of
them since the colored-solver-default lane (five after the A7 lane, three before it). All 32 of the
A6 lane's sites (2026-09-17) landed here, and so did the L2 calibration's seven `miri-slow:` sites,
which is why this figure moved and the two above it did not. The two remaining `cfg_attr` sites are
not Miri's: `profiling/reduce.rs`'s
`not(debug_assertions)` and `tb_neg_m2w_block_reference.rs`'s `not(all(miri, feature = …))`. None of
the 149 belong to either leg above.

**Leg: physics release — the debug-ignored `slow:` tests.** Six tests are ignored in every debug
build and under Miri by `#[cfg_attr(any(miri, debug_assertions), ignore = "slow: …")]`, so none of
the four commands above runs them: G2, G6, G7, A7-R1, A7-R2 and `simd_solve_on_off_bit_identical` in
[`crates/boyko_physics/tests/sleep_settles_box_piles.rs`](crates/boyko_physics/tests/sleep_settles_box_piles.rs)
— box piles through the real physics schedule, A7-R1 being the only scene-level gate on a resting
pile's creep (defect A7). Their leg is the physics release run:

```powershell
cargo test --release -p boyko-physics --no-fail-fast
```

In it `sleep_settles_box_piles` must print `running 13 tests` and `12 passed; 0 failed; 1 ignored`
(measured 2026-09-19 and again 2026-09-21 on the union; the one ignore is its `generator:`); that
binary took ~80 s, ~73 s of it A7-R1 (msvc, 2026-09-18), and 141 s on 2026-09-21 — wall time, not a
gate. The whole leg on 2026-09-21: 52 binaries, **505 passed; 0 failed; 3 ignored** — the
`deferred:` AVX2 signed-zero proptest, the `generator:` flicker histogram, and the narrowphase lane's
`jolt_pyramid_parallel_narrowphase_is_bit_identical`, a **plain** `slow:` site this leg does NOT
run: its own reason says "run in release with `-- --ignored`", so its leg is
`cargo test --release -p boyko-physics --test narrowphase_parallel_equivalence -- --ignored`, the
second plain `slow:` site in the tree after `boyko_app/tests/app12_timer_resolution.rs`.
`-- --ignored` in a debug build is NOT the six's leg: the debug build is exactly where
they are ignored, and it would run them unoptimized.

**At least 24 ignored tests belong to no leg at all**, and must not be swept into one. Six were
enumerated when this section was written: three *generators* that assert nothing and emit source to
paste (`dump_maximal_frame_barrier_stream`, `dump_vb_unsplit_barrier_streams`,
`dump_vb_split_barrier_streams` — the second's own doc warns that running it casually re-measures
the baselines the split is compared against), two *deferred milestones* that are RED by design until
M2's JCGT cubic lands (`brick_field_is_conservative_lower_bound`,
`trilinear_reconstruct_is_a_tight_lower_bound_in_r1`), and one *timing probe* documented "NOT a CI
gate" (`no_starvation_every_worker_makes_progress`). None of those six carries a vocabulary prefix.
**A further 18 sites do carry one** (17 `deferred:`, 1 `generator:`, measured 2026-09-18 after the
A7 merge), in `boyko_ecs` (9), `boyko_serialize` (5), `boyko_physics` (2 — the flicker generator and
the AVX2 signed-zero proptest; the A7 lane's four red-first tests are no longer plain ignores), and
`boyko_ui` (2); each is red or silent by design and belongs to no routine leg either. 24 is a floor,
not a census: the 147 plain reasons that carry no prefix have not been classified.

⚠️ **The device-free leg is one test, and it is an explicit invocation rather than a filter,
because the partition CANNOT be derived from the reason strings.** A keyword classifier over
`{GPU, RTX, Vulkan, windowed, device, dispatch}` put 10 on the device-free side — of the 143 plain
sites the tree held when the experiment was run, 183 today — and **8 of those 10 are wrong**, wrong
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
against **183** plain sites today.
The migration has started at the sites, not in this list: **36 of the 183 plain reasons already
carry a prefix** — 17 `deferred:`, 16 `gpu-windowed:`, 2 `slow:`, 1 `generator:` (measured
2026-09-21 on the union; 31 of 178 on 2026-09-18 after the boot-validation merge, whose ten new
device tests all carry `gpu-windowed:`; the light-table lane then added four `gpu-windowed:` and
the narrowphase lane one `slow:`, so every plain site the two lanes brought arrived prefixed; the
A7 merge before them removed four `deferred:` plain sites by resolving them) — and the other 147
do not, the same 147 as on 2026-09-18. The 16 `gpu-windowed:` are `boot_validation_clean.rs` 7,
`unwritten_shadow_map_gate.rs` 4, `sdf_marcher_sun.rs` 3, `forward_teardown_destroys_forward_sets.rs`
1, `vb_teardown_destroys_boot_resources.rs` 1. So the migration is still mechanical *per site*, and
afterwards each leg is a `grep` and every new ignore picks its own leg at the site; what it is not
is bookkeeping already done.

The `cfg_attr` side is further along and has already outgrown the list. Of the 150 (measured
2026-09-21): **92 `miri-slow:`, 19 `instrument:`, 9 `tractability:`, 6 `miri-unsupported:`, 6
`slow:`, 1 `miri-arm:`, 17 with no prefix.** `instrument:` (the `boyko_threadpool` `block.rs`
recording-allocator tape), `tractability:` and `miri-arm:` — **29 sites** — are prefixes the closed
vocabulary above does not contain, and the census's own error text uses `tractability:` as its
example; the census enforces non-emptiness only, so nothing flags them. Either the list grows or
those sites are re-prefixed — a ruling to take before the partition becomes a `grep`, not
bookkeeping.

⚠️ **One prefix is already being used against its own definition, and a mechanical leg built from
it would run those tests in the wrong place.** `slow` is defined above as *device-free, wall-clock
budget*, i.e. a `--ignored` leg — but `sleep_settles_box_piles.rs`'s G2, G6 and G7 spell
`#[cfg_attr(any(miri, debug_assertions), ignore = "slow: …")]`, and that file's module header says
the opposite for them: they run in the ordinary **release** test run, and `-- --ignored` in a debug
build is NOT their leg. The two **plain** `slow:` sites (`app12_timer_resolution.rs`,
`narrowphase_parallel_equivalence.rs`) use the prefix as defined; the six `cfg_attr(any(miri,
debug_assertions))` `slow:` sites do not — the same word names two legs. Harmless while the census
only enforces non-emptiness; it bites the day the partition becomes a `grep`. Whichever ruling
makes the prefixes mechanical has to say which class a release-only-but-device-free test takes.

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
