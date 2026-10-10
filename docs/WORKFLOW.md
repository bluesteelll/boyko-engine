# Workflow rationale — the orchestrator's reference

This file holds the orchestrator-facing rationale moved out of [CLAUDE.md](../CLAUDE.md) to keep the
always-loaded file small; CLAUDE.md holds the rules. Every section below is CLAUDE.md text moved
verbatim, from one of two sources. From CLAUDE.md as it stood before `b0b4ec3f` (2026-10-07):
Build-command flags, Russian docs freeze (all but its last sub-bullet), rust-analyzer and semble;
Token economy was written for this file by that commit. From the integration trunk's CLAUDE.md at
`68ec9312`, which added or rewrote them after the two lines forked at `49f2fcfb`: Gate temp
directory, Lockfile, Feature axis, Ignored suite, Model routing and the last sub-bullet of Russian docs
freeze. Only relative link targets were re-based from the repository root to `docs/`, and the
feature-axis heading was raised one level. Where a passage restates a rule, the shortened rule in
CLAUDE.md is authoritative. Numbers are as measured on the dates they carry and are not re-derived
here. A cross-reference that leaves its section ("the four commands above", "rather than only here")
points into CLAUDE.md as it stood before the move.

## Build-command flags

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

## Gate temp directory

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

## Lockfile

**`Cargo.lock` is tracked (2026-09-24), and CI runs every cargo build with `--locked`.** While it was
git-ignored, each worktree and each CI run resolved its own dependency versions, and a `TypeId`
literal follows the resolved graph: UG-15 leg (2) went RED on a tree byte-identical to a green one,
and swapping the two worktrees' locks turned it green. **A worktree created before this commit holds
an untracked `Cargo.lock`: move it aside before merging a commit that tracks it.**

## Feature axis

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
failure this gate exists to stop.** 29 non-default features across 14 packages: CI's own derive
step, run against the tree on 2026-10-09, prints `[feature census] 29 non-default features
declared, 6 refused, 23 legs to run (0 of them at a narrowed target scope)`.

- **Covered — 23 legs, run on `ubuntu-latest` at `--all-targets`:** `boyko-app/{hwrt,
  profiling-census}`; `boyko-diag/section-gate`; `boyko-ecs/{bench-alloc, big_query_table,
  profiling-analysis}`; `boyko-log/test-probe`; `boyko-physics/{bench-alloc, bp-query-counts,
  narrowphase-counts}`; `boyko-render/{hwrt, profiling-census, reflect, test-readback}`;
  `boyko-scene/reflect`; `boyko-threadpool/scheduler-trace`; `boyko_rhi_vulkan/{goldens, hwrt,
  profiling-census, spec_constant_smoke}`; `boyko_shaderdsl/emit`; `reflect-dogfood/reflect`;
  `reflect-fixture/reflect`. `boyko-render/test-readback` is a leg but **adds no coverage of its
  own**: the crate's self-referential dev-dependency turns that feature on in every `--tests` build.
  MEASURED 2026-10-09 by running each leg from a Windows host with
  `--target x86_64-unknown-linux-gnu`, the CI `RUSTFLAGS` and `--keep-going`: 21 legs green with
  0 warnings. The two `bench-alloc` legs cannot be cross-checked that way, because
  `libmimalloc-sys`'s build script needs an `x86_64-linux-gnu-gcc`, which the runner has and a
  Windows box does not. Both were green natively on msvc (2026-09-18). The tests behind
  `boyko-physics/{bp-query-counts, narrowphase-counts}` run in the `feature-count-tests` job on
  `windows-latest`. That job was the last step of `feature-legs` until 2026-10-10.
- **NOT covered — 6, each with its reason in `ci.yml`:** `bench-bevy-vs-boyko/{bench-alloc,
  nightly}` — the bevy tree, excluded from every job but `bench-compile`;
  `boyko-threadpool/tb-neg-m2w` — `compile_error!` outside Miri, by design, gated instead by
  `scripts/tb_neg_gate.{ps1,sh}`; `boyko_sdf_math/nightly` and `boyko_shaderdsl/nightly` —
  **KNOWN-RED, an open defect** (below); `boyko-app/profiling-alloc` — **KNOWN-RED, an open defect
  and not a platform judgement**: the feature's `#[global_allocator]`
  (`crates/boyko_app/src/profiling/alloc_shim.rs:145`) collides with the one that the
  `harness = false` test `crates/boyko_app/tests/e1_engine_ui_alloc_census.rs:271` installs in a
  binary linking `boyko_app`, so `--all-targets` fails with that single error, identically on Linux
  and on Windows (MEASURED 2026-10-09). Until 2026-10-09 the table refused
  `boyko-app/{hwrt, profiling-alloc, profiling-census}` because *that crate's lib did not build off
  Windows*; that reason is gone (below).
- **What no leg covers at all:** clippy lints on gated code. The legs are `cargo check`, so rustc
  warnings fail them (CI's `RUSTFLAGS` carries `-D warnings`), but no clippy lint has ever run over
  a `#[cfg(feature = …)]` item in this tree. Narrowed here, not closed.

⚠️ **Three of those legs exist only because a refusal row was re-measured and withdrawn, and a
refusal that overstates its reason is the same defect as a coverage claim that overstates its
coverage.** `boyko-render` was excluded as a crate that "does not compile for a non-Windows target
at all", on a measurement of **2 errors**. RE-MEASURED 2026-09-18 on the same triple, those 2 were
**one** `error[E0601]` — `main` function not found in `examples/orbit_cube_window.rs`, which was
then `#![cfg(windows)]` as a whole crate — plus cargo's own summary line (`--keep-going`
confirms no second failing target hides behind it). The **lib compiles**:
`cargo check -p boyko-render --features hwrt --lib --tests --target x86_64-unknown-linux-gnu` exits
0 in 10.8 s from a clean `cargo clean -p boyko-render`. The false row had excluded three legs and
left **34 `#[cfg(feature = "hwrt")]` sites** under `crates/boyko_render/src` compiled by nothing —
the exact class this job was created to close, one crate over from where it was found. `ci.yml`
therefore carries a **second table beside the refusals**: a leg whose *lib* builds on the runner but
whose *example* does not gets a narrower TARGET SET, never an exclusion. An exclusion is only for a
crate whose lib itself cannot compile there. The scope table has had no rows since 2026-10-09: the
example has a `main` on every target (`crates/boyko_render/examples/orbit_cube_window.rs:682-693`),
so the three `boyko-render` legs run at `--all-targets`; the mechanism stays for the next crate whose
lib builds there while an example does not. Both tables fail the derive step when a row outlives its
subject, and the leg floor (`MIN_LEGS`) is pinned at 15, below the 23 derived.

Locally a feature is checked per crate — this is the whole recipe:

```powershell
cargo check -p boyko_rhi_vulkan --features hwrt --all-targets    # the leg that was red for 32 days
cargo check -p boyko-app        --features hwrt --all-targets    # its cfg(windows) arm: no CI job
cargo check -p boyko-render     --features hwrt --all-targets    # the example's cfg(windows) loop
```

VERIFIED 2026-09-18 on `stable-x86_64-pc-windows-msvc`: all three lines are green (0 errors,
0 warnings), and so were `boyko-app`'s `profiling-alloc` and `profiling-census` at `--all-targets`;
`profiling-alloc` has failed there since `5eab1d0b` (2026-09-23) added the e1 census test and its
own allocator (the refusal above). CI compiles `boyko-app/{hwrt, profiling-census}` and the example on
Linux. Since 2026-10-10 its test-running jobs run on `windows-latest`, and they compile `boyko-app`'s
GPU path and the example's window loop at **default** features. What depends on this by-hand recipe
is the `#[cfg(windows)]` arm of each feature, and for those arms it is the **only** compile they get.
No CI job runs it, so it is a recipe, not coverage.

⚠️ **Three features are RED right now, and the job refuses them by name rather than hiding them:**
`boyko-app/profiling-alloc` (the allocator collision above) and these two.
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

⚠️ **The identical blind spot exists on the PLATFORM cfg, and for `boyko-app` it is narrowed, not
closed.** One `cargo` invocation compiles one target triple, so a `#[cfg(not(windows))]` item is
exactly as invisible as a feature-gated one — and it has already produced the same defect:
`crates/boyko_app/src/diag.rs:184` spelled `codes::E3004.number()` inside a
`#[cfg(not(windows))]` reporter. It landed 2026-08-13 (`b809041e`), **four days before** the sweep
that fixed its siblings (`718a5129`), and that sweep — which worked off rustc's own E0308 spans on a
Windows box — could not be shown it. RE-MEASURED 2026-09-18 with `--keep-going`:
`cargo check -p boyko-app --lib --target x86_64-unknown-linux-gnu` failed on DEFAULT features with
five printed diagnostics — 2 × E0432 (`crate::gpu_scene` is a `#[cfg(windows)]` module, then
imported unconditionally at `runner.rs:77` and `particle_readback.rs:34`) and 3 × E0308, all at
`diag.rs:184` — which rustc's summary line counts as **"7 previous errors"**. The failure was in the
**lib**, so the ubuntu runner could not build `boyko-app` at all, and no CI leg covered its cfg arms.

**Repaired 2026-10-09 by the Linux build lane: `boyko-app` compiles on Linux; no GPU path.** The
reporter passes the typed `codes::E3004` (`diag.rs:190`), and the `gpu_scene` imports are
`#[cfg(windows)]` (`runner.rs:77-82`, `particle_readback.rs:35-36`). Off Windows there is no GPU path
to run: `gpu_scene`, the host and its dump and probe drivers are `#[cfg(windows)]` modules
(`crates/boyko_app/src/lib.rs:60-115`), the frame loop is the `#[cfg(windows)]` `run_windowed`
(`runner.rs:237-238`), and its non-Windows twin reports E3004 and returns `AppExit(true)`
(`runner.rs:1022-1026`). One layer down, `boyko_rhi_vulkan`'s non-Windows loader is `None`
(`crates/boyko_rhi_vulkan/src/device.rs:1740-1743`) and its non-Windows `Window::open` returns
`UnsupportedPlatform` (`crates/boyko_rhi_vulkan/src/window.rs:775-781`). Until 2026-10-10 the
mirror image stayed open: every CI job ran on `ubuntu-latest`, so **no job compiled a
`#[cfg(windows)]` item**, and that whole path was compiled only on a Windows box.

**Since 2026-10-10 (owner decision, release PR #4), every CI job that executes a test binary runs on
`windows-latest`**, the platform the engine is developed, measured and pinned on. Those jobs are
`test`, `profile-census`, `reflect-on`, `reflect-census`, `reflect-dogfood`,
`feature-count-tests` and `force-alloc-panic`. They compile that path at default features and run
its device-free tests. Two kinds of job stay on `ubuntu-latest`, both blocking. The first is the
build-only jobs: `check`, `clippy`, `profile-legs`, `feature-legs` and `bench-compile`. They keep
the engine compiling for Linux. The second is `miri` and `loom`, which pass there. What is still
open is narrower. No clippy lint reaches a `#[cfg(windows)]` item, and no feature leg compiles one.
CI run 4 (38026578745) ran the whole debug and release selection on `ubuntu-latest`, and its reds
fell in three classes of pin taken on Windows: allocation counts, float bytes across C
runtimes, and timing on a 4-vCPU runner. One more red, `boyko-physics --bench narrowphase_classes`,
was no pin: it failed on the Windows MSVC dev host with the same numbers, because its
`touching == manifolds` assert predated V2's speculative contacts, and the bench now runs the
overlap-only rule it was written for. So the Linux test run is kept as `test-linux`, an
informational job (`continue-on-error: true`). Its header in `ci.yml` lists each red, and making
those pins hold on Linux is a follow-up.

**`boyko-render` was not in that position, and saying it was is the error the scope table above
corrected.** Its lib builds on the runner, and its feature legs compile it, `cfg(not(windows))` arms
included. Until 2026-10-09 the one thing the runner could not build was
`examples/orbit_cube_window.rs`, then `#![cfg(windows)]` as a whole and so without a `main` there.
Its window loop is now `#[cfg(windows)] mod windowed` (`orbit_cube_window.rs:56-57`), and a
non-Windows `main` prints a notice and exits (`:687-693`), so `--all-targets` builds the example on
the ubuntu runner. Since 2026-10-10 the `windows-latest` test jobs compile the loop at default
features. No job compiles it with a feature on.

⚠️ **Until 2026-10-09 one consequence reached past the feature axis.** The `check`, `test`,
`profile-legs`, `clippy`, `force-alloc-panic` and `bench-compile` jobs all ran `--workspace` on
`ubuntu-latest`. `test` and `force-alloc-panic` have run on `windows-latest` since 2026-10-10.
Their target sets include `boyko-app`'s lib (all six) and that example (the five `--all-targets`
jobs). A failing unit makes cargo exit non-zero, so **none of those jobs could
pass on the runner.** That was inferred from the two per-crate measurements plus the command lines.
`bench-compile`'s default target selection is the non-obvious one, and it was checked:
`cargo +nightly bench --no-run --workspace --exclude boyko_demo --target x86_64-unknown-linux-gnu
-Z unstable-options --unit-graph` lists `boyko_app`'s lib and no example. The repair landed in those
two crates, not in the gate. MEASURED 2026-10-09 from a Windows host with
`--target x86_64-unknown-linux-gnu`, the CI `RUSTFLAGS` and `--keep-going`: every cargo job in
`ci.yml` exits 0 at its own selection (`test` and `bench --no-run` measured as `check`, `miri` as a
stable `check` of its packages), except the two `bench-alloc` legs that host cannot build. That
measurement is a compile, not a run of any test.

## Ignored suite

The four commands above run **none** of the `#[ignore]`d tests.
[tests/ignore_reasons_census.rs](../tests/ignore_reasons_census.rs) prints what exists on every run
(`cargo test -p boyko-engine --test ignore_reasons_census -- --nocapture`). Measured 2026-10-06 on
`u/phys-sr` at its sync with the trunk `c335b4c6` (first parent `aa00faf6`; rung D-M0 adds one `solo` site to B3's
`7d5a0015` reading, `u/fix-cq-sb`, L9 C4, PC-24, L10b and W8S each add one `.rs` file, L10 adds six, C1 adds five, none of those adds a site; DM1 adds nineteen `.rs` files and twelve `gpu-windowed` sites; tree C4 adds one `miri-slow` site; V2 adds four `.rs` files, five `slow` sites (the fidelity gate's four, a device-free release leg, and G6's V2 twin) and one `miri-slow` site; SR adds seven `.rs` files and two `miri-unsupported` sites, T6's ladder bound and failure B's 250 ms hold, both wall-clock), it read:

```text
[ignore census] 377 sites (208 plain, 169 cfg_attr) across 12 crates, 1869 .rs files walked, 0 waivers
[ignore classes] <none>=1, deferred=19, feature+gpu=1, feature+gpu-cap=3, feature+gpu-windowed+gpu-cap=4,
  feature+miri-slow=1, flaky=1, generator=7, gpu=29, gpu-cap=1, gpu-windowed=132, gpu-windowed+gpu-cap=1,
  miri-slow=132, miri-unsupported=29, slow=14, solo=2; scopes: miri-only=160, native=216, release-only=1;
  rule inputs: 128 sites call a device entry, 2 exist only under Miri, 9 are feature-conditioned
```

⚠️ **Every count in this section is a snapshot, and the census's printed lines are the only
figures to quote — read the run, not this paragraph.** `MIN_SITES` is a floor, so a prose count
can drift arbitrarily far and stay green. How the count moved (164 → 280 → … → 355 → 356 → 368 → 369 → 375 → 377) is in
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
not the grep. The counts in the comments were taken at `7d5a0015` and read the same at `79005dfd`;
the device-free leg's comment also gives its count after rung D-M0, and every other count reads the
same at the `u/d-m0` merge.

**Leg: device-free ignored tests** — the plain `solo`/`slow` sites. Any machine, no GPU.

```powershell
rg -n '^[^/]*#\[ignore = "(solo|slow):' -g '*.rs'      # 5 sites on u/d-m0 (4 at 7d5a0015)
```

Each site's reason names its command (a `solo` site adds `--test-threads=1`). The two `solo` sites:

```powershell
cargo test -p boyko-log --test l14_sink_policy -- --ignored --test-threads=1
cargo test -p boyko-ecs --lib ecs::memory::component_pool::tests::commit_floor_tests::os_truth_row1_default_pool_at_full_capacity -- --ignored --exact --test-threads=1
```

Each must print `running 1 test`. The second is rung D-M0's commit-floor oracle at a default pool's
full capacity (G3 row 1 of the packing plan). It is `solo` for its cost in commit charge, about
768 MiB of process-wide Windows commit, not for its time. It is selected with `--exact` because a
bare `-- --ignored` over the lib binary would also sweep in any ignored lib test that lands later.

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
[`crates/boyko_physics/tests/sleep_settles_box_piles.rs`](../crates/boyko_physics/tests/sleep_settles_box_piles.rs)
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
`miri-unsupported` site (`rg -n '^[^/]*ignore = "(feature\+)?miri-' -g '*.rs'`, 182 on
2026-10-10, after the Miri sweep's tests were skipped or budgeted under Miri — the sweep is `.github/workflows/miri-sweep.yml`, weekly and not a merge gate; 158 at `7d5a0015`); almost all are `cfg_attr(miri, …)` and run *natively* in the ordinary legs. Two tests
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
[`crates/boyko_symcensus/ug15/receipts/b3/ignore_classes.tsv`](../crates/boyko_symcensus/ug15/receipts/b3/ignore_classes.tsv).
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

## Model routing

**Model routing — the current routing** (owner decision "both steps", 2026-09-30; rulings 5 and 7 of
the 2026-09-30 list in the section "After window 9a (2026-09-30)" of
[`docs/physics/perf-campaign/levers/00-RULINGS.md`](physics/perf-campaign/levers/00-RULINGS.md)).
The `model:` frontmatter of all nine agent files stays `opus`; the orchestrator's lane scripts
choose the tier per call, as below.

- **`tester`: Sonnet 5.5**, from the lanes started after 2026-09-30. The lanes already running then
  (tree-c4, v2-spec) were not switched mid-flight. Its GREEN is still crossed by the Opus
  `code-reviewer`, which runs beside it.
- **`developer`: Opus for kernel, `unsafe` and numerics work; Sonnet for mechanical work** — re-pins
  by rule, doc and registry merges, window records.
- **Sonnet 5.5 also takes** scouting and inventory, window preparation, the transcription of
  finished results into docs, and re-runs of existing gates with exit-code reports, each checked by
  an Opus verifier or a hard oracle (ruling 5 (a)).
- **Opus: `architect`, `architecture-critic`, `code-reviewer`, `results-analyst` and every
  verifier.** `researcher`, `project-analyst` and `doc-writer` keep their frontmatter, Opus; nothing
  in the rulings moves them.
- **`effort: low` on Opus is NOT used.** On the retrieval benchmark below it is the worst arm: 4
  confidently wrong answers, against Sonnet 5.5's 1.
- **The escape-rate watch.** A Sonnet tester GREEN that is later overturned by the code-reviewer or
  by a window is logged. One escape that Opus would have caught reopens this routing.
- **A tier change needs a measured diff:** at least ~25 items, with *confident-but-wrong* scored
  separately from *missed*. The routing above rests on the two measurements of 2026-09-30; any
  further move needs its own.

**The measurements of 2026-09-30 — what moved the routing.** Both were run on this repository's own
material, and both score the confident-but-wrong answer apart from the miss, because that is the
failure a downstream reader cannot detect.

*The retrieval benchmark, re-run.* The 2026-09-07 benchmark below was repeated exactly: the same 27
ground-truthed questions, the same brief and the same tree snapshot (`c20f6883`), on three arms.

| arm | found | rank-1 | TIGHT MRR | **confidently WRONG** | tokens per task (cache reads) |
|---|---|---|---|---|---|
| Sonnet 5.5 | 26/27 | 24 | 0.907 | **1** | 475k |
| Opus 5.5 | **27/27** | **26** | **0.975** | **0** | 475k |
| Opus 5.5, `effort: low` | 23/27 | 23 | 0.852 | **4** | 313k |
| (2026-09-07 Sonnet, for comparison) | 23/27 | 22 | 0.833 | 4 | — |

- Sonnet 5.5 closed most of the old gap (4 → 1 confidently wrong). The remaining difference is one
  question, which n = 27 cannot separate from noise; Sonnet's one confident miss named the wrong
  file.
- Sonnet does **not** use fewer tokens per task (475k, the same as Opus). Any saving is the
  per-token weight against the subscription limit, not the volume.
- `effort: low` on Opus is the **worst** arm on this faculty: 34 % fewer tokens and 4 confidently
  wrong. The 2026-09-07 advice below to "optimise effort instead of the tier" is refuted for search
  and verification.

*The judgment benchmark.* Retrieval is not the faculty a `tester` or a `code-reviewer` uses
(catching a gate that cannot fail, a false claim), so 47 historical cases with known answers were
mined from this repository's own post-mortems and fix commits: vacuous gates 8, false claims 13,
vacuous passes 6, wrong unit 4, wrong tree 2, `unsafe` / ordering 3, other 11. The reviewer reads
the pre-fix commit through `git show`, and a model-blind Opus judge scores the answers.

| arm | caught | missed | confidently wrong | tokens per task (reads) |
|---|---|---|---|---|
| Sonnet 5.5 | 45/47 | 1 | 1 (a vacuous gate: a trybuild floor that does not guard the glob) | 1.37 M |
| Opus 5.5 | **46/47** | **0** | 1 (a unit case: a GCD quantum read as an upper bound) | 1.56 M |

- Sonnet 5.5 used 13 % fewer tokens per task here.
- ⚠️ **Ceiling effect.** The task points at the artifact, so it measures spotting a defect given its
  place, not discovery among many artifacts. Both benchmarks together put Sonnet 5.5 within one or
  two answers of Opus on n = 27 + 47.

**History: the measurements of 2026-09-07.** From 2026-07-26 until 2026-09-30 every role ran on Opus
(owner decision, 2026-07-26), and from 2026-09-07 there was a MEASURED diff behind it rather than
only a judgement. The two measurements below are kept as they were written; their "sonnet" is the
Sonnet of that date, and for Sonnet 5.5 they are superseded by the re-run above.

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

**So, on 2026-09-07, the downgrade question was answered in the negative, with numbers, for the one
role that could be measured cheaply — and the roles above it are protected LESS, not more.** Only
facts are gated downstream, never judgement: `cargo` catches code that does not compile but not UB
in an `unsafe` block; a `tester`'s pass/fail is gated while a test that *cannot fail* reads as
green; a `doc-writer`'s output is gated by nothing at all (repairing doc rot introduced new
falsehoods in 3 of 5 measured attempts). Do not downgrade any role on the argument that it is
"mechanical" — that premise was tested here and failed. **Optimise effort, batch size and redundant
arms instead of the tier** (`effort:` is a per-call knob on Opus and keeps its judgement). ⚠️ The
effort advice is refuted for search and verification by the `effort: low` arm of 2026-09-30 above,
and the argument against "mechanical" still stands: the 2026-09-30 move rests on a measured diff,
not on that argument.

The previous split put `developer`, `tester`, `researcher`, `doc-writer` and `project-analyst` on Sonnet as "mechanical / gathering" roles. That premise did not survive contact with this codebase: the roles it called mechanical are the ones that catch the campaign's defects. On the VB-SV0 stage alone, implementers refuted the orchestrator's own prescriptions **nine times** — a ULP tolerance that was wrong in form because the leaf ends in a cancellation, a `-D`-push route replaced by an `OpArrayLength` bound the orchestrator had not considered, a z-range check whose prescribed site would have panicked at boot in every process, and a NaN claim corrected on the wrong leaf. None of that is transcription work. Do not downgrade any role without a measured before/after quality diff — this applies to all nine now, not only to `code-reviewer` (which remains the last line against `unsafe`/atomics UB). The orchestrator itself stays on the session model.

## Token economy

Measured 2026-10-07: 1,148 agents, 66.5k calls; data `D:/tmp/phys-orch/token-audit/`.

Cost ≈ turns × context: ~95 % of volume is cache re-reads, and 79 % of what is re-read is context growing inside ONE
agent. The NUMBER of agents is not the driver (spawn + brief + re-orientation ≈ 5 %), and every modelled stage merge
cost MORE (+0.4 … +8.6 %), because the merged agent re-reads the earlier stage's context on every later turn.

- **Do not merge stages to save agents.** Merging also kills independent refutation (tester ∥ reviewer, triage → fix).
  Parallel arms buy wall time, not tokens: run them when their inputs or roles differ.
- **Parallel checkers start from one context pack, not N explorations.** Reviewers spend 42–48 % of their tokens
  before their first action, triage 61–65 %, cut 58 %. When ≥ 2 agents of a run need the same orientation, one scout
  writes a ≤ 15k-token map (files + line ranges, key facts, the delta) to disk and every brief points at it; the
  checkers stay separate agents. The shared prompt prefix (system, CLAUDE.md, brief) is already served from one cache.
- **Keep the cache warm.** Subagents use the 1-h cache TTL (`subagentPromptCacheTtl` in `.claude/settings.json`);
  23.5 % of cost was full re-writes after waits past the old 5-min TTL. Launch a long gate detached in its own console
  (`Start-Process … -WindowStyle Minimized`; never `run_in_background`/`&` — a reaped shell's children die 0xC0000142),
  have it write PID / DONE / exit codes / a per-row summary, and wait in bounded slices that check the PID is alive and
  the gate is within its time budget; past budget is a hang — stop it by its own PID.
- **Logs live on disk, not in context.** Tool outputs over 5k tokens, re-read for the rest of an agent's life, are
  27.9 % of all re-reads. Use `rg -n` + ranged reads (≤ ~200 lines); read summaries, verdict lines and failure excerpts,
  never whole logs. Check `test result:` targets BY NAME against the expected set (equal counts over different sets are
  a vacuous green).
- **Hand off through files.** Briefs carry paths + section names, never pasted documents; every report is saved;
  a resume = a commit + a ≤ 1-page `handoff.md`. Split a long agent only at a commit, past ~350k context.
- **One owner per heavy gate per round.** Developer: check/clippy, its red-firsts, the gates its diff touches.
  Tester: workspace tests, pins, Miri, anchors. (Both ran a workspace check in 21 of 38 lanes.)
- **Rounds ≥ 2 are delta-scoped**: the confirmed items' red-first on parent and HEAD, ≥ 2 own mutations aimed at the
  fix delta and at the gates the last round added (S5 r2's only defect came from one), the touched gates, workspace
  check/clippy. One full workspace run on the lane's final HEAD before merge.
- **Keep every checking arm** (critique, triage, tester mutations, review each round — together ≈ 9–10 % of cost):
  triage confirmed a defect in all 13 rounds; SR review r3 caught a liveness defect after a green tester. Cut an arm
  only on a measured escape-rate diff over ≥ ~25 items. Lower effort was measured and rejected.
- **Small always-loaded floor.** Each 1k tokens in CLAUDE.md / MEMORY.md costs ≈ 0.29 % of all spend: MEMORY.md
  ≤ ~15 KB, measurement tables live in docs; typed agents start at 40–48k vs `claude` 72–89k — prefer typed agents.
- **Orchestrator.** Workflows return verdicts + paths (≤ ~2k tokens), not full agent texts; read reports by section;
  restart from the memory checkpoint past ~400k context.

## Russian docs freeze

- ⚠️ **`docs/ru/` is FROZEN as of 2026-09-07 — owner decision — and its pairing rule is WITHDRAWN.** The English documents are the source of truth and are the only side maintained. Do not update the Russian side when the English one changes, and do not translate new documents into it.
  - The freeze was taken with its cost stated and accepted: the pair **will** diverge, which is exactly what the withdrawn rule warned about — a reader cannot tell which side is current and finds out only by acting on the stale one. That is why the freeze is announced in [`docs/ru/README.md`](ru/README.md) itself, where the reader lands, rather than only here.
  - It began from a synchronised state: both sides were last written by `efd7735f` (2026-09-03), verified by `git log -1` per path, so the divergence is measurable from that commit forward rather than unknown.
  - A future session must NOT "repair" the divergence by re-syncing or by deleting the directory. Either is an owner call, not a tidy-up.
  - One addition inside the directory postdates the rule it would have followed but predates the freeze, and is kept: `docs/ru/README.md`'s section on twin checks testing SAMENESS rather than TRUTH (lane commit `d272e1fd`, 2026-08-29, +21 lines). It reached the integration line through the A6 reflection merge with no conflict raised — that line had not touched the file since the merge base, so git took the lane's side silently — while two reports said *"the frozen directory held"* on the strength of the one file there that did hold (MEASURED 2026-09-22). A report about one file of this directory is not a report about the directory. The A8 merge (the `integ/unified` cut) kept that section, unchanged, below the freeze announcement: re-syncing it and dropping it are both the owner's calls, not a merge's.

## rust-analyzer

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
an `#[allow(clippy::disallowed_types)]` sweep, a string inside a shader — and `semble` (below) when
the question is about meaning in prose rather than about one symbol.

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

## semble

Rationale in this repo lives in Markdown and in long comment blocks, so the answering words often
are not the asking words. `semble` (machine-local, `pip install --user "semble[mcp]"`) indexes
`.md` and `.rs` — tree-sitter chunking carries comment nodes verbatim — and re-validates its cache
on **every** search, so it cannot go a month stale the way a manually-rebuilt index can.

Use the wrapper, [`tools/semsearch.py`](../tools/semsearch.py), rather than `semble` bare — it adds
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
