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

**`Cargo.lock` is tracked (2026-09-24), and CI runs every cargo build with `--locked`.** While it was
git-ignored, each worktree and each CI run resolved its own dependency versions, and a `TypeId`
literal follows the resolved graph: UG-15 leg (2) went RED on a tree byte-identical to a green one,
and swapping the two worktrees' locks turned it green. **A worktree created before this commit holds
an untracked `Cargo.lock`: move it aside before merging a commit that tracks it.**

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

The four commands above run **none** of the `#[ignore]`d tests.
[tests/ignore_reasons_census.rs](tests/ignore_reasons_census.rs) prints what exists on every run
(`cargo test -p boyko-engine --test ignore_reasons_census -- --nocapture`). Measured 2026-09-24 on
the trunk at the `u/phys-thinbox` merge (first parent `68437dfe`), it read:

```text
[ignore census] 355 sites (191 plain, 164 cfg_attr) across 12 crates, 1823 .rs files walked, 0 waivers
[ignore classes] <none>=1, deferred=19, feature+gpu=1, feature+gpu-cap=3, feature+gpu-windowed+gpu-cap=4,
  feature+miri-slow=1, flaky=1, generator=7, gpu=29, gpu-cap=1, gpu-windowed=120, gpu-windowed+gpu-cap=1,
  miri-slow=130, miri-unsupported=27, slow=9, solo=1; scopes: miri-only=156, native=198, release-only=1;
  rule inputs: 127 sites call a device entry, 2 exist only under Miri, 9 are feature-conditioned
```

⚠️ **Every count in this section is a snapshot, and the census's printed lines are the only
figures to quote — read the run, not this paragraph.** `MIN_SITES` is a floor, so a prose count
can drift arbitrarily far and stay green. How the count moved (164 → 280 → … → 355) is in
`git log -p CLAUDE.md`, not here.

Every site states its requirement **and its class**, and the census fails the build if a new one
does not — a bare `#[ignore]` is the **third** way to make a check disappear, after `unsafe` and
`#[allow(clippy::disallowed_types)]`, and it carries a written rationale like the other two.

**The class prefix (enforced since rung B3, 2026-09-23).** Every reason reads
`#[ignore = "<class>: <prose>"]`, `class` from a closed vocabulary, each naming a leg:

- `gpu` — a headless Vulkan device; `gpu-windowed` — a device plus a window and swapchain (a
  desktop session); `gpu-cap` — hardware with an optional capability the prose names (RT, ray
  query, `VK_KHR_pipeline_executable_properties`);
- `feature` — the test does not exist, or does not run, without `--features <name>`, which the
  prose names;
- `solo` — device-free, needs `--test-threads=1` (process-wide state); `slow` — device-free,
  wall-clock budget;
- `miri-slow` — runs natively, and Miri would finish it only given hours; `miri-unsupported` —
  runs natively, and Miri cannot execute what it needs at all (a child process, a custom
  `#[global_allocator]`, a deliberate leak);
- `generator`, `deferred`, `flaky` — no leg (below).

The only multi-class spellings are `feature+<class>` and `gpu-windowed+gpu-cap` (so
`feature+gpu-windowed+gpu-cap` too): one requirement set, one spelling, one grep. `generator`,
`deferred` and `flaky` stand alone. A missing prefix, a prefix outside the list or a spelling
outside these is RED, with one exemption: a site ignored only in native release runs in every debug
run and names no leg, so its reason may carry no class. A prefix outside the list is still RED
there. That scope is the `release-only=1` above and the `<none>=1` class count (today
`crates/boyko_app/src/profiling/reduce.rs`'s `debug_assert!` guard test). The census also
refuses a class that the site's own source contradicts:

- a `cfg_attr` site is ignored only where its predicate holds, and the census evaluates the
  predicate in native debug, native release and Miri — a site ignored only under Miri takes
  `miri-slow` or `miri-unsupported` and nothing else;
- a test whose body reaches a device (`boot*_or_skip`, `VulkanContext::boot`,
  `EnginePlugins::window`, `with_windowed_present`, or a same-file helper reaching one) carries a
  `gpu*` class, or a no-leg one;
- a test that exists only under Miri carries a `miri-*` class, and a `miri-*` class needs `miri`
  in the site's `cfg` context;
- a test a cargo feature conditions carries `feature` and names its `--features <name>`, and every
  `--features` name in any reason must be declared in the crate's `[features]`, so a renamed
  feature reds.

**So every leg is now a `grep` by prefix, not a reading.** The patterns below skip comment lines
(`^[^/]*`), and on the tree above each matched the census's own class counts; quote the census,
not the grep. The counts in the comments were taken at `7d5a0015` and read the same at `79005dfd`.

**Leg: device-free ignored tests** — the plain `solo`/`slow` sites. Any machine, no GPU.

```powershell
rg -n '^[^/]*#\[ignore = "(solo|slow):' -g '*.rs'      # 4 sites at 7d5a0015
```

Each site's reason names its command (a `solo` site adds `--test-threads=1`), for example:

```powershell
cargo test -p boyko-log --test l14_sink_policy -- --ignored --test-threads=1
```

Select by name (`--exact <test>`) wherever a binary also holds a no-leg class: `ui_s0_measure.rs`
carries one `slow` gate beside two `generator` harnesses, and a bare `-- --ignored` sweeps the
generators in. The output must read `running N tests` for the N you selected. A `running 0 tests`
line is a vacuous pass, not a pass — see the `#![cfg(miri)]` trap below, which produced exactly
that.

**Leg: device-needing ignored tests** — every class with a `gpu` part. A machine with the GPU;
the orchestrator or the owner runs it, per binary with `--test-threads=1`.

```powershell
rg -n '^[^/]*ignore = "(feature\+)?gpu' -g '*.rs'             # the whole leg (159 at 7d5a0015)
rg -n '^[^/]*ignore = "(feature\+)?gpu(-cap)?:' -g '*.rs'     # headless, no window (34)
rg -n '^[^/]*ignore = "(feature\+)?gpu-windowed' -g '*.rs'    # need a desktop session (125)
rg -n '^[^/]*ignore = "[a-z+-]*gpu-cap' -g '*.rs'              # need RT / ray query / exec-props (9)
```

`rg -n '^[^/]*ignore = "feature\+' -g '*.rs'` lists every site that needs a cargo flag, across
legs (9 at `7d5a0015`: 8 device sites and the tb-neg Miri arm below). A `feature+` device site is
not even *compiled* without its flag, which its prose names — `--features hwrt` ×7,
`--features spec_constant_smoke` ×1. Most device sites also sit behind
`#![cfg(windows)]` and vanish on Linux. There is no single command — each binary has its own
env-var protocol in its module header (`BOYKO_DISABLE_VALIDATION`, `BOYKO_HZB_DUMP`,
`BOYKO_WINDOW_FRAMES`, …); a driver spawns its own workers, and a worker run alone returns at
once.

**Leg: physics release — the debug-ignored `slow:` tests.** Six tests are ignored in every debug
build and under Miri by `#[cfg_attr(any(miri, debug_assertions), ignore = "slow: …")]`, so none of
the four commands above runs them: G2, G6, G7, A7-R1, A7-R2 and `simd_solve_on_off_bit_identical` in
[`crates/boyko_physics/tests/sleep_settles_box_piles.rs`](crates/boyko_physics/tests/sleep_settles_box_piles.rs)
— box piles through the real physics schedule, A7-R1 being the only scene-level gate on a resting
pile's creep (defect A7). The attribute is multi-line there, so the grep is too
(`rg -U 'cfg_attr\(\s*any\(miri, debug_assertions\),\s*ignore = "slow:' -g '*.rs'`, 6 at
`7d5a0015`). Their leg is the physics release run:

```powershell
cargo test --release -p boyko-physics --no-fail-fast
```

In it `sleep_settles_box_piles` must print `running 13 tests` and `12 passed; 0 failed; 1 ignored`
(measured 2026-09-19 and again 2026-09-21 on the union; the one ignore is its `generator:`); that
binary took ~80 s, ~73 s of it A7-R1 (msvc, 2026-09-18), and 141 s on 2026-09-21 — wall time, not a
gate. `-- --ignored` in a debug build is NOT these six's leg: the debug build is exactly where they
are ignored, and it would run them unoptimized. The plain `slow:` site
`jolt_pyramid_parallel_narrowphase_is_bit_identical` is not in this run either — its reason says
`cargo test --release -p boyko-physics --test narrowphase_parallel_equivalence -- --ignored`, which
is the device-free leg above, in release.

**Leg: Miri.** `cargo +nightly-x86_64-pc-windows-msvc miri test` skips every `miri-slow` and
`miri-unsupported` site (`rg -n '^[^/]*ignore = "(feature\+)?miri-' -g '*.rs'`, 158 at
`7d5a0015`); almost all are `cfg_attr(miri, …)` and run *natively* in the ordinary legs. Two tests
exist only under Miri — `miri_fixed_loop.rs`'s plain site and `miri_phase19.rs`'s
`miri_cascade_wide_path`, both in `#![cfg(miri)]` files — and run under Miri with `-- --ignored`.
The `feature+miri-slow` site is the tb-neg Tree-Borrows arm in
`boyko_threadpool/tests/tb_neg_m2w_block_reference.rs`: ignored natively and under a plain Miri
run, it runs only through `scripts/tb_neg_gate.sh` (Miri, `--features tb-neg-m2w`,
`--include-ignored`), and its success is an abort.

**No leg: `generator`, `deferred`, `flaky`** (`rg -n '^[^/]*ignore = "(generator|deferred|flaky):'
-g '*.rs'`, 27 at `7d5a0015`). They must not be swept into a leg. *Generators* assert nothing
beyond an instrument sanity check and emit an artifact: the three barrier-stream dumps
(`dump_maximal_frame_barrier_stream`, `dump_vb_unsplit_barrier_streams`,
`dump_vb_split_barrier_streams` — the second's own doc warns that running it casually re-measures
the baselines the split is compared against), `ui_s0_measure.rs`'s two print-only harnesses, the
flicker histogram and the link-configuration table. *Deferred* tests are red by design until a
named ballot or milestone lands (M2's JCGT cubic for `brick_field_is_conservative_lower_bound` and
`trilinear_reconstruct_is_a_tight_lower_bound_in_r1`, Gaia F4 and GK-2, AB-11, …). The one *flaky*
site is the timing probe `no_starvation_every_worker_makes_progress`, documented "NOT a CI gate".

**One site carries no class:** `boyko_app/src/profiling/reduce.rs`'s
`cfg_attr(not(debug_assertions), …)`. It is ignored only in release, where the `debug_assert!` under
test does not exist, and runs in every debug run; it names no leg, and the census exempts that
scope alone.

⚠️ **The partition cannot be derived from the reason strings — which is why the prefix exists, and
why it was assigned by reading.** A keyword classifier over
`{GPU, RTX, Vulkan, windowed, device, dispatch}` put 10 on the device-free side — of the 143 plain
sites the tree held when the experiment was run — and **8 of those 10 were wrong**, wrong in the
direction that produces a green:

- `negative_chained_barrier_hazard` and `a5_gpu_off_vs_on_wall_clock_ab` **do** need a device; their
  reasons name the *hazard* and the *purpose*, not the requirement. Both call `boot_*_or_skip`, so
  on a GPU-less box they return early and **pass** — a leg that includes them reports green while
  measuring nothing.
- `miri_app_driver_substeps_and_hold` is inside a `#![cfg(miri)]` file, so natively it does not
  exist. Filtering for it prints `running 0 tests` and exits 0. **Measured while writing this leg.**
- The other five are the generators / deferred / flaky above, which would pass meaninglessly, fail
  outright, or flake.

Since B3 the partition is the enforced prefix set. All 355 sites were classified by reading what
each test needs — its body, its boot call, its `cfg`, its module header — and the per-site record
(class, action, one line of evidence) is
[`crates/boyko_symcensus/ug15/receipts/b3/ignore_classes.tsv`](crates/boyko_symcensus/ug15/receipts/b3/ignore_classes.tsv).
Both traps above are now red by rule, not by vigilance: a `solo`/`slow` class on a test that boots
through `boot_*_or_skip` fails the census, and so does a test in a `#![cfg(miri)]` file without a
`miri-*` class. What the rules cannot see — a device reached only through a child process or
another module's helper, or the choice between `solo` and `slow` — stays a reading.

**The rulings B3 took (2026-09-23) to make the prefixes mechanical** — mechanism, not values:

1. **The list does not grow.** The five out-of-list prefixes the tree held were re-prefixed by
   reading each site: `tractability:` (9) → `miri-slow:`, the same meaning; `instrument:` (19) →
   `miri-unsupported:` (the recording `#[global_allocator]` is `cfg(not(miri))` and
   `Recording::start` panics under Miri; one site re-executes a child process); `miri-arm:` →
   `feature+miri-slow:`; `M2:` (2) → `deferred:`; `calibration:` → `generator:`.
2. **The predicate conditions the class**, which settles the warning this section carried that
   `slow` named two legs. A plain `slow:` site is the device-free `--ignored` leg; a `slow:` inside
   `cfg_attr(any(miri, debug_assertions), …)` is ignored in debug only, and its leg is the ordinary
   `--release` run. The attribute form tells them apart, and a Miri-only `cfg_attr` may take only
   `miri-slow` or `miri-unsupported`.
3. **Composite spellings instead of a precedence order** (`feature+<class>`,
   `gpu-windowed+gpu-cap`), with the three no-leg classes standing alone.
4. **`feature` means "not compiled, or not run, without the flag"**, checked both ways against the
   `cfg` the census reads. A test that compiles without the flag but is meaningful only with it
   (`grand_showcase_mvpm`, which needs `--features hwrt` and ray query) is `gpu-windowed+gpu-cap`
   and names the flag in its prose.

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

**Model routing** (set via each agent's `model:` frontmatter): **every role runs on Opus** — owner decision, 2026-07-26, and as of 2026-09-07 there is finally a MEASURED diff behind it rather than only a judgement.

**The measurement (2026-09-07).** 27 retrieval questions over this tree, each with a ripgrep-verified
ground-truth passage and phrased WITHOUT the target's own distinctive vocabulary, run through an
identical brief on three tiers. The task is the most *mechanical* thing an agent here ever does —
search, read, report a file and line span — so it is the friendliest possible case for a cheaper tier:

| tier | found | rank-1 | TIGHT MRR | **confidently WRONG** | returned nothing |
|---|---|---|---|---|---|
| haiku | 14/27 | 14 | 0.519 | **8** | 3 |
| sonnet | 23/27 | 22 | 0.833 | **4** | 0 |
| opus | **27/27** | **25** | **0.957** | **0** | 0 |

The decisive column is the fourth, not the first. Sonnet asserted "I found it" on **four** questions
where its answer was wrong; Opus did so **zero** times. A confident wrong answer is this
repository's own catalogued failure mode, and it is the one a downstream reader cannot detect.

⚠️ **A six-question pilot of the same experiment said Sonnet TIED with Opus (6/6 each) and was
wrong** — those six happened to be questions Sonnet gets right. n=6 could not separate the tiers;
n=27 separated them by four answers and by all of the false confidence. Any future routing
measurement needs at least ~25 items, and must score *confident-but-wrong* separately from *missed*.

**The second measurement — the development workflow itself, from this repository's own history.**
The roles named below ran on Sonnet until 2026-07-26 and on Opus after, one repo, one orchestrator:
a natural before/after. Matched 43-day windows either side of the switch (545 vs 470 commits):

| signal | Sonnet era | Opus era | ratio |
|---|---|---|---|
| `fix:` commits — actual defect repairs | 10.6% | 10.6% | **1.00×** |
| commit-subject length — the style confounder | 10.4 words | 14.7 words | 1.41× |
| **numeric corrections** per 100 commits ("19 stale sites, not the 9 reported") | 4.0 | 13.4 | **3.32×** |
| **self-refutations** per 100 commits (the workflow catching its OWN output) | 7.9 | **50.9** | **6.45×** |

Commit style did become more discursive, which is why the raw "admits something was wrong" rate
proves nothing on its own — but self-refutation grew **4.6× faster than subject length**, so prose
cannot explain it. And the repair rate is *identical*: the workflow does not ship more fixes, it
finds far more falsehood in what it and its predecessors had already written — "the gate that passed
its own red mutation", "the gate caught its own author twice", "two of Rev 10's own repairs were
wrong, and the review caught both", "3 of 5 repairs first made it worse". The matched Sonnet window
contains **two** self-refutations, one of them a plain race fix.

⚠️ Confounded, and the confounder is named: the Opus era coincides with campaigns *about* gate
quality (diagnostics ladders, the anchors gate, VG critique rounds), which generate self-refutation
by their nature. Topic selection is part of the effect; the 6.45× is not attributable to the tier
alone. What survives the confounder is the pairing with the retrieval table above — **two
independent measurements, one synthetic and one historical, both isolating the same faculty: not
capability, but self-scepticism.** Opus's edge is knowing when its own output is wrong, which is
precisely the axis this repository's entire failure taxonomy runs along ("a check that could not
fail", "green from emptiness", "a confident wrong answer").

**So the downgrade question is answered in the negative, with numbers, for the one role that can be
measured cheaply — and the roles above it are protected LESS, not more.** Only facts are gated
downstream, never judgement: `cargo` catches code that does not compile but not UB in an `unsafe`
block; a `tester`'s pass/fail is gated while a test that *cannot fail* reads as green; a
`doc-writer`'s output is gated by nothing at all (repairing doc rot introduced new falsehoods in 3
of 5 measured attempts). Do not downgrade any role on the argument that it is "mechanical" — that
premise was tested here and failed. **Optimise effort, batch size and redundant arms instead of the
tier** (`effort:` is a per-call knob on Opus and keeps its judgement).

The previous split put `developer`, `tester`, `researcher`, `doc-writer` and `project-analyst` on Sonnet as "mechanical / gathering" roles. The previous split put `developer`, `tester`, `researcher`, `doc-writer` and `project-analyst` on Sonnet as "mechanical / gathering" roles. That premise did not survive contact with this codebase: the roles it called mechanical are the ones that catch the campaign's defects. On the VB-SV0 stage alone, implementers refuted the orchestrator's own prescriptions **nine times** — a ULP tolerance that was wrong in form because the leaf ends in a cancellation, a `-D`-push route replaced by an `OpArrayLength` bound the orchestrator had not considered, a z-range check whose prescribed site would have panicked at boot in every process, and a NaN claim corrected on the wrong leaf. None of that is transcription work. Do not downgrade any role without a measured before/after quality diff — this applies to all nine now, not only to `code-reviewer` (which remains the last line against `unsafe`/atomics UB). The orchestrator itself stays on the session model.

## Orchestration discipline

- **Clarify before acting.** If a request is ambiguous, or you do not fully understand the intended scope/behavior, **ask** (`AskUserQuestion`) or **enter Plan Mode BEFORE any Write/Edit** — never guess. Only VALUES/SCOPE calls go to the owner; decide perf/architecture forks yourself, with numbers.
- **Plan-Mode threshold.** Any change touching **≥3 files**, or that you cannot describe in one sentence, goes through Plan Mode (`ExitPlanMode` approval) before the first edit.
- **Backstop hook.** A `UserPromptSubmit` hook ([.claude/hooks/clarify_gate.py](.claude/hooks/clarify_gate.py)) injects a reminder when a prompt is imperative but names no file/path/symbol. It reminds; it never blocks. Subagents cannot ask the user — a subagent that hits an ambiguity **stops and escalates to the orchestrator** (already encoded in `developer.md`).
- **graphify-first, one retry.** The PreToolUse hooks ([graphify_read_gate.py](.claude/hooks/graphify_read_gate.py), [graphify_bash_gate.py](.claude/hooks/graphify_bash_gate.py)) nudge `graphify query/explain/path` before reading/grepping source. graphify is tuned to the ECS kernel; if it returns off-target or empty results, fall back to Grep/Read **once** — do not retry graphify.

## Communication

- Chat messages between Claude and the user can be in Russian.
- **Every artifact written into the repository is in English**: code, doc comments, inline comments, commit messages, internal docs, agent prompts, mdBook content, audit reports — everything. No mixed-language files.
- **ONE EXCEPTION: [`docs/ru/`](docs/ru/).** Owner-granted 2026-08-02. That directory holds Russian versions of documents the owner reads himself. The files there are NOT a rule violation and must not be "fixed" back to English. Everything outside it stays English, including the originals.
- ⚠️ **`docs/ru/` is FROZEN as of 2026-09-07 — owner decision — and its pairing rule is WITHDRAWN.** The English documents are the source of truth and are the only side maintained. Do not update the Russian side when the English one changes, and do not translate new documents into it.
  - The freeze was taken with its cost stated and accepted: the pair **will** diverge, which is exactly what the withdrawn rule warned about — a reader cannot tell which side is current and finds out only by acting on the stale one. That is why the freeze is announced in [`docs/ru/README.md`](docs/ru/README.md) itself, where the reader lands, rather than only here.
  - It began from a synchronised state: both sides were last written by `efd7735f` (2026-09-03), verified by `git log -1` per path, so the divergence is measurable from that commit forward rather than unknown.
  - A future session must NOT "repair" the divergence by re-syncing or by deleting the directory. Either is an owner call, not a tidy-up.
  - One addition inside the directory postdates the rule it would have followed but predates the freeze, and is kept: `docs/ru/README.md`'s section on twin checks testing SAMENESS rather than TRUTH (lane commit `d272e1fd`, 2026-08-29, +21 lines). It reached the integration line through the A6 reflection merge with no conflict raised — that line had not touched the file since the merge base, so git took the lane's side silently — while two reports said *"the frozen directory held"* on the strength of the one file there that did hold (MEASURED 2026-09-22). A report about one file of this directory is not a report about the directory. The A8 merge (the `integ/unified` cut) kept that section, unchanged, below the freeze announcement: re-syncing it and dropping it are both the owner's calls, not a merge's.

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

`rust-analyzer` serves every `.rs` file through the `LSP` tool, declared by a machine-local plugin at
`~/.claude/skills/rust-analyzer-local/.lsp.json`. Prefer it for anything about a symbol's identity or
its relations, because it answers out of the compiler front-end instead of an index that can be
stale:

- `goToDefinition` / `findReferences` / `goToImplementation` — exact and workspace-wide.
- `hover` — type, doc comment, and the **computed** layout (`size = 312 (0x138), align = 0x8, no
  Drop`), which is load-bearing in a codebase whose principles are about layout.
- `workspaceSymbol` — locate a type or function by name across every crate.
- `incomingCalls` / `outgoingCalls` — call hierarchy, for tracing a hot path.

Use `Grep` (it is ripgrep) when the question is literal or pattern-shaped — a `// SAFETY:` census,
an `#[allow(clippy::disallowed_types)]` sweep, a string inside a shader — and graphify when it is
architectural rather than about one symbol.

### Meaning in prose and comments — `semble`, and where it beats ripgrep

Rationale in this repo lives in Markdown and in long comment blocks, so the answering words often
are not the asking words. `semble` (machine-local, `pip install --user "semble[mcp]"`) indexes
`.md` and `.rs` — tree-sitter chunking carries comment nodes verbatim — and re-validates its cache
on **every** search, so it cannot go a month stale the way a manually-rebuilt index can.

Use the wrapper, [`tools/semsearch.py`](tools/semsearch.py), rather than `semble` bare — it adds
the cross-encoder rerank stage that carries most of the measured value, and it prints both orders:

```
python tools/semsearch.py "<question>" <tree> -k 8
```

Bare `semble search` also works; its `--content` defaults to **code only**, so pass `all` (or
`docs`) there or prose is silently not searched.

**The configuration is a measured optimum over 27 ground-truthed questions, not a guess.** Every
number below is `hits@10` on the strict metric (the returned span must actually overlap the target):

| configuration | hits@10 | recall@40 |
|---|---|---|
| semble as shipped — chunk 750, no rerank | **8** | 11 |
| chunk 1500, fused rerank | 12 | 14 |
| **chunk 4500, pool 40, fused at α=0.5** ← the wrapper's defaults | **16** | **17** |

**hits@10 doubled and recall went 11 → 17**, from a chunk constant, a pool cap, an 80 MB
cross-encoder and a fusion weight — no new model, no torch, no npm. Four things were each measured
and each is counter-intuitive, so do not "simplify" them away:

- **The chunk budget is the biggest single lever, and the shipped default is the worst setting.**
  750 chars (~190 tokens) against rationale blocks averaging ~2,200 splits a block into ~4
  fragments, which semble's own ranker then penalises (2nd fragment from a file ×0.5, 3rd ×0.25).
  Swept: 750 is last of {750, 1500, 3000, 4500, 6000} on every metric. The wrapper patches the
  constant at call time rather than editing `site-packages`, because a patched install is silently
  reverted by the next upgrade.
- **A deeper pool is worse AND dearer.** Recall does not improve past 40 (5/6 at 40, 150 and 400),
  while reranking degrades monotonically and cost grows 10× (27 s → 272 s/query). This reproduces
  arXiv 2411.11767's finding that a pointwise reranker beats the retriever alone in barely half of
  measured cases at high K.
- **Fusion is required; both extremes lose.** At chunk 4500, hits@10 is 11 with the first stage
  alone, 14 with the cross-encoder alone, and **16 fused at α=0.5**. Replacing the order outright
  moved one target from rank 38 to 2 while pushing a rank-1 target down to 9 — which is why the
  wrapper prints `was=#N`, and why `--no-rerank` exists.
- **Do not change the embedding model.** The default `potion-code-16M-v2` measured best;
  `potion-retrieval-32M` and `potion-base-32M` both scored worse and dropped a code target out of
  the top 40 entirely, and merging all three candidate pools recovered no recall the default did not
  already have.

**The split is measured, not assumed (2026-09-07), and it goes both ways:**

- **Broad topical question → semble.** "Which subsystem decides shadow-map stability" against
  ripgrep is `shadow` = **6,265 occurrences in 270 files**, useless as a starting point; semble
  returned 8 ranked hits including the right plan document and the doc comment recording why the
  shadow-map camera once pointed 180° away.
- **Specific rationale with a guessable rare word → ripgrep.** "Why can the yield count not be
  tuned" was found by `tune|tuned|tunable` scoped to one crate — **3 files**, answer among them —
  while semble **missed it in the top 5 even in the tree that holds it**.
- Rule of thumb that follows: if a keyword guess would return few files, grep; if it would return
  hundreds, let semble rank.

⚠️ **A bigger embedding model is not the fix, and it was tested twice.** On a single question
`potion-retrieval-32M` (249 MB) looked better than the default and `potion-multilingual-128M`
(1002 MB) looked worse; on the six-question set the default won outright. The reason is structural:
every one of them is a *static* embedding — token vectors summed with no attention — so a paraphrase
bridge like "cannot be tuned" ⇒ "no constant threshold can certify it" is out of reach **by
construction**, and semble loads only `model2vec.StaticModel`. That is the ceiling a cross-encoder
steps over, because it reads the query and the chunk together.

The failure was diagnosed before it was fixed, and the diagnostic is reusable: a **near-verbatim**
query retrieves the target at rank 1 with score ~**0.0197** — measured three times, on three
different targets — while a paraphrase of the same question leaves it outside the top 5 at ~0.009.
So the chunk is indexed and recall is adequate; only the ordering fails. **Treat a first-stage top
score below ~0.012 as "nothing really matched"** and prefer a ripgrep guess on a rare word from the
question. Semble's own `rerank` is a lexical heuristic (file/identifier boost, path penalties over
an RRF fusion with BM25) and is already on — it is not a cross-encoder, so the two stack.

⚠️ `semble mcp` does not exist — it exits 2. The CLI satisfies the integration need on its own.
Shader grammars are absent (`wgsl`, `hlsl` warn and fall back to line chunking).

⚠️ **The server covers ONLY the projects listed in `settings["rust-analyzer"].linkedProjects`, and
that key REPLACES project auto-discovery rather than extending it.** Two consequences, both measured
2026-09-07:

- A file in a worktree that is not listed gets **syntax only**: `documentSymbol` answers in full
  while `hover`, `findReferences` and `workspaceSymbol` come back **empty**. That empty answer is
  indistinguishable from "no such symbol", so it is a silent wrong answer, not an error.
- The main checkout must never be dropped from the list, or fixing a lane breaks what already
  worked.

The list therefore carries the main checkout plus whichever lanes are under work, and lanes are
removed as they finish: each project costs several GB of RSS, growing with query volume. When two
trees are linked, `workspaceSymbol` returns one hit per tree for identically-named packages —
tell them apart by path form, since a linked lane answers with an absolute path and the main
checkout with a relative one.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
