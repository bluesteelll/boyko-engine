//! Jolt parity pyramid: the FIXED-WINDOW RUNNER.
//!
//! This is the runner of the physics perf campaign's P0 (`docs/physics/perf-campaign/01-PLAN-REV1.md`
//! §1–§2 and implementation step 6, amended by `00-RULINGS.md`). It replaces the Criterion bench
//! that stood here, whose timed window was broken (H1): 20 warm steps, then time-sampled iterations
//! on a world that was never reset, so the stretch of the collapse it timed depended on W and
//! `T(1)/T(W)` compared different windows. `docs/MEASUREMENT-QUEUE.md` §10 is the run sheet that
//! drives it.
//!
//! # What one run is
//!
//! **One world per process** (rulings, open question 1): the profiler's store binds one world per
//! process and refuses a second with `E9204`, while that world's fold then returns silently. So the
//! runner builds exactly one world, the measured one, and never a throwaway (the Criterion bench's
//! `build(1)` anti-vacuity world is gone; the body count is checked on the measured world).
//!
//! The world is spawned, then stepped `--steps` times from t = 0 with **no warm-up**, one `Instant`
//! pair around each `Schedule::run` and nothing else inside it. That is Jolt's own timed window
//! (`PerformanceTest.cpp`: 500 `Update`s from `StartTest`, each timed). The runner reports Jolt's
//! metric over `--window` (steps / Σ t) and writes every step's wall time to the CSV, from which
//! the driver takes sub-windows such as [0, 100) and [100, 500).
//!
//! Everything the runner reads between steps — the profiler fold, the receipts (manifold and pair
//! counts, the top box's height, the awake count) and the anti-vacuity recount — happens AFTER the
//! step's `Instant` pair closes. The disarmed and armed runs read the same receipts, so the only
//! difference between them is the arming, the fold and the per-zone snapshot.
//!
//! # Scenes (`--scene`)
//!
//! | scene | bodies | layer gap (`--gap` default) | friction | what it is |
//! |---|---|---|---|---|
//! | `jolt` | 1240 + floor | 0.5 | 0.2 | Jolt's `PyramidScene.h`, index for index |
//! | `rest` | 1240 + floor | 0 | 0.5 | `benches/sleeping_pipeline.rs`'s `pyramid_sleeping_off` pile: the same pyramid exactly touching, the A7-R1/R2 scene |
//! | `s16` | 16 + floor | 0 | 0.5 | a single tower of 16 of the same boxes: the small-scene regression guard |
//!
//! Transcribed from Jolt v5.3.0 (`PerformanceTest/PyramidScene.h`, `PerformanceTest.cpp`): a
//! static floor box of half-extents (50, 1, 50) at (0, −1, 0); boxes of half-extent 1 with no
//! convex radius, `cBoxSize = 2`, `cBoxSeparation = 0.5`, `cPyramidHeight = 15`; the placement
//! loop including the odd-layer half-box offset; gravity (0, −9.81, 0); `dt = 1/60` with one
//! collision step per `Update`. Friction is Jolt's default 0.2 on the `jolt` scene (H2): Jolt
//! combines √(0.2·0.2) and boyko max(0.2, 0.2), which are equal. Jolt's 0.05 damping is removed on
//! Jolt's side by the parity patch (H3), because boyko has no rigid-body damping and adding an
//! engine feature to match a benchmark was rejected. The `jolt` scene holds exactly 1240 dynamic
//! bodies, checked on the measured world: a drift there means the two engines no longer run the
//! same scene.
//!
//! # Configurations (`--cfg`, `--solver`)
//!
//! The runner sets every knob it depends on explicitly, so a change of a shipped default (the
//! colored solver and `simd_solve` became the defaults in `56c1e9e7`, 2026-09-18) cannot change
//! what a row measures:
//!
//! * `--cfg a` (cfg-A, H7): the colored solve; `parallel_solve = W > 1` (`--parallel-solve` could
//!   force it on at W = 1 until L4 retired J-P1); `parallel_narrowphase` follows `parallel_solve`
//!   (L5; from L5 on cfg-A parallelises the narrowphase at W > 1, so a P0 cfg-A row is comparable
//!   only as the "before" of that pairing); `broadphase = AllPairs` under `Manual` selection;
//!   `simd_solve` off.
//!   `parallel_broadphase` follows `parallel_solve`, as in the Criterion bench, and does nothing
//!   here: it is read only on the `Grid` path, and there only from `MIN_PARALLEL_BODIES` = 4096
//!   bodies up (tree report C1 — the Criterion bench's comment claiming otherwise was wrong).
//! * `--cfg as` (cfg-As, the L11 `J-As` row): cfg-A plus `simd_solve`, i.e. AllPairs with the AVX2
//!   cohort kernel on and `parallel_solve = W > 1`. It isolates the solve kernel from the
//!   broadphase (cfg-B changes both), which is what L11's per-stage gates are read on. Bit-identical
//!   to cfg-A by the O7 bit suite, so it must end in cfg-A's pose bytes too.
//! * `--cfg b` (cfg-B): cfg-A plus `Grid` plus `simd_solve`. Both changes are bit-identical by
//!   construction (`production_grid_equals_all_pairs`; the O7 bit suite), so cfg-A and cfg-B must
//!   end in equal pose bytes: `--pose-out` on one run and `--expect-pose` on the other assert it.
//! * `--cfg default` (the default): `PhysicsConfig::default()` as this tree ships it, except that
//!   `--parallel-solve` forces its knob on and `--sleeping on|off` sets its. The `rest` rows use
//!   it. Since L4
//!   that default has `parallel_solve` on, so `--parallel-solve` no longer changes a `--cfg
//!   default` row, and such a row at W ≥ 2 dispatches its wide colors on its own; since L5 C4 it
//!   has `parallel_narrowphase` on too, so such a row also dispatches its narrowphase at W ≥ 2
//!   (the W4 check reads the flag from the built config, so the `NP_*` expectations follow).
//! * `--parallel-np on|off` sets `parallel_narrowphase` under every `--cfg`, after the rest: the
//!   same-binary A/B of the parallel narrowphase (L5), e.g. cfg-A at W = 8 with the solve parallel
//!   and the narrowphase serial.
//! * `--broadphase allpairs|tree|grid` sets `broadphase` under every `--cfg`, after the rest, and
//!   pins `broadphase_select` to `Manual` so the policy cannot override it: the same-binary A/B of
//!   the tree broadphase (gate G5 of `docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md`,
//!   commit C3). All three kinds emit the same pair set, so the pose bytes must be equal across
//!   them (`--expect-pose`). The Tree runs its brute all-pairs loop at or below
//!   `BroadphaseTree::brute_max_rows()` rows (printed in the summary as `tree_brute_max_rows`), so
//!   on `s16` a `--broadphase tree` row is the brute path; the per-step check derives the path
//!   from the kind, the row count and that threshold.
//! * `--solver reference` wires `add_physics_systems::<SoftStepSolver>` instead of
//!   `add_physics_colored_solve` (R-ref, which prices D1). It takes `--cfg default` only, and no
//!   sleeping, parallel solve or canary, none of which exists on that path.
//!
//! **J-P1 is retired (L4; lever rulings, L5 W2).** The solve now decides once per step that a
//! one-worker pool runs inline, so `--parallel-solve` at W = 1 opens no scope while `waves` still
//! counts every wide color: a J-P1 row would report ω₁ ≈ 0 with no void. `validate` refuses the
//! combination, and ω₁ comes from a zero-work spawn/join microbench from L4 on.
//!
//! `substeps`, `relax_iterations`, `simd` and the soft-contact constants stay at the tree's
//! defaults and are printed in the summary.
//!
//! # The profile (`--arm-profiler`)
//!
//! An armed run binds the process's profiler to the measured world, inserts and arms the store,
//! and folds after every step, outside the timed pair. Every system span and every physics zone
//! (`boyko_physics::profiling`) is then diffed per step and written to the CSV in nanoseconds
//! (the clock's calibrated ticks-per-ns is printed). The driving thread claims a diagnostics lane
//! first, because the fold's own `__fold` span is pushed from it and a push from an unclaimed
//! thread is counted as a dropped sample.
//!
//! **Per-step anti-vacuity (ruling W4).** Each step, every zone's sample count is compared with
//! its structural expectation, recomputed from the world rather than from the solver's columns:
//! each system 1; `phys_solve_build`, `phys_restitution`, `phys_store`, `phys_write_back` 1;
//! `phys_gravity`, `phys_warm_apply`, `phys_integrate`, `phys_pass_biased` = substeps;
//! `phys_pass_relax` = substeps × relax; `phys_color_wide` / `phys_color_narrow` = (wide / narrow
//! colors) × sweeps, the class recomputed from `ConstraintGraph` and `Manifolds` with the frozen
//! islands' manifolds excluded exactly as the solve excludes them; the sleep zones 1 / 2 / 1 when
//! sleeping is on — except on a step of the colored solve's no-awake fast path (L10 C3a: sleeping
//! on, no dynamic row awake, no slot solved, all recomputed from the world), which opens neither
//! freeze span, no substep or color span and no restitution span, and whose count must equal the
//! solver's own `fast_path_steps` over the run; `phys_sleep_classify` and the `phys_sleep_held`
//! counter (the held rows) 1 each
//! when the step's sleep-skip mode is `Sets` (L10: colored, sleeping on, `--sleep-skip` unset or
//! `sets`); `phys_np_dispatch`, `phys_np_compact` and `phys_np_axis_commit` 1 each when the
//! recomputed narrowphase chunk count C is at least 2, else 0; each counter once, with its value
//! equal to the recomputed slots, pairs, manifolds, points and C. C is recomputed by
//! [`expected_np_chunks`], this runner's copy of the narrowphase's chunk rule, from the exported
//! `NP_*` constants, the step's pair count, `--workers` and `parallel_narrowphase` — the lanes
//! term included (lever ruling W1), so a narrowphase that dispatched on one worker voids the row. A step that differs is VOID: the CSV marks it, the summary names the first
//! one, and the process exits 3. On the reference solver every in-solve zone must read zero.
//! A disarmed run must push nothing at all into any lane (the ring traffic over every lane and both
//! regions is compared before and after), or it too is void.
//!
//! Also per step, armed: `waves` (wide color spans: the solve's `pool.scope` dispatches when
//! `parallel_solve` is on and W ≥ 2 — the span is opened by color class, not by dispatch, so at
//! W = 1 it counts colors that ran inline), the executor gap `g = wall − Σ system spans`, the
//! unzoned residue `u = solve span − Σ in-solve zones` and `r = Σ pass spans − Σ color spans`
//! (plan §2 identity, O2). The driver applies the closure rules to them; the runner only
//! reports.
//!
//! # Thread counts (rulings, open question 2)
//!
//! boyko at `--workers W` runs a pool of W worker threads, plus the thread that calls
//! `Schedule::run`, which becomes the pool's dispatcher. A system that is not exclusive runs on a
//! worker, and while it runs the dispatcher is parked in the executor's step 5 (`schedule.rs`,
//! "park until a worker unparks us"): it does not steal the solver's chunk tasks, whose joiner is
//! the worker running the solve. Jolt at `-t=W` runs W − 1 job threads plus the calling thread,
//! which executes jobs while it waits on a barrier. So both run W threads through the parallel
//! work. The runner does not assume this; it records a witness per armed step: the samples pending
//! on the dispatcher's lane against the physics zones the solve pushed. If the solve ran on the
//! dispatcher, all of those samples land on its lane, and `threads.solve_on_dispatcher_steps` in
//! the summary counts such steps. A step of the colored solve's no-awake fast path (L10 C3a) is
//! not counted: it pushes too few solve samples for the comparison to say where the solve ran.
//!
//! # The canary (`--canary-frac F --canary-ref-ns T`)
//!
//! Adds one system, `.after(narrowphase).before(build_graph)`, that spins F × T ns (J-C: F = 0.05
//! and T the pass's J-A mean step time at the same W, which the driver passes). Every other
//! physics system is ordered against it, so it sits on the critical path at every W: the step's
//! wall time must rise by the spin, and its own system span must read it. The ordering uses the
//! stage indices `add_physics_colored_solve` returns: `PhysicsStageKeys` documents each as its
//! stage's `SystemKey` inner index, and `SystemKey`'s field is public while the type is not
//! nameable outside the kernel, so the key is built by writing that index into a copy of the
//! canary's own key.
//!
//! # Flags
//!
//! ```text
//! --scene jolt|rest|s16        required; without it the binary runs its self-check (below)
//! --workers W                  pool size (default 1)
//! --steps N                    steps from spawn (default 500)
//! --window A..B                the summary's window, A <= B <= N (default 0..N)
//! --gap G                      layer gap (default per scene)
//! --solver colored|reference   (default colored)
//! --cfg a|as|b|default         (default default)
//! --parallel-solve             force parallel_solve on; since L4 a no-op wherever it is accepted
//!                              (cfg-A/B at W > 1 and --cfg default have it on), refused at
//!                              W = 1 (J-P1 is retired, see "Configurations")
//! --parallel-np on|off         set parallel_narrowphase (L5), under any --cfg
//! --contact-reuse on|off       set contact_reuse (L9b), under any --cfg; either value also reads
//!                              the narrowphase's pair classes after every step (untimed) into the
//!                              summary's `pair_classes`, as does any row whose config has
//!                              contact_reuse on without the flag
//! --reuse-distance D           contact_reuse_distance τ in metres (with --contact-reuse on)
//! --broadphase allpairs|tree|grid
//!                              set broadphase (tree broadphase C3) under Manual selection, under
//!                              any --cfg
//! --sleeping [on|off]          set sleeping (L10 C1a). A bare --sleeping is `on`, the spelling
//!                              every recipe recorded before C1a uses. Unset, the --cfg decides:
//!                              cfg-A/As/B off, --cfg default the tree's PhysicsConfig default
//! --sleep-skip off|sets        set PhysicsConfig::sleep_skip (L10 C2a), with sleeping on only.
//!                              Unset, the tree's default (sets). Until L10 C3b holds islands
//!                              the two give one hash
//! --threshold T                sleep threshold (speed², with --sleeping on)
//! --frozen-by K                void unless every dynamic row is frozen on step K (R-S: 300;
//!                              with --sleeping on)
//! --arm-profiler               the armed profile run
//! --canary-frac F              with --canary-ref-ns T: the canary spins F*T ns
//! --canary-ref-ns T
//! --csv PATH                   the per-step CSV
//! --pose-out PATH              write the final pose bytes (every dynamic body's full state)
//! --expect-pose PATH           compare the final pose bytes with a file; exit 4 if they differ
//! --label TEXT                 echoed into the summary
//! --bench                      ignored (cargo bench passes it)
//! ```
//!
//! # Output
//!
//! * stdout: a few readable lines, then one line `SUMMARY {json}` for the driver. The pose hash is
//!   FNV-1a 64 over the final pose bytes: every dynamic body's `RigidBody` (position, linear
//!   velocity, rotation, angular velocity) as little-endian `f32` bits, in spawn order. The
//!   summary's `config` carries `tree_brute_max_rows`, and `broadphase_tree` carries the tree
//!   broadphase's cumulative `TreeDiag` after the last step (all zero unless the kind is `Tree`):
//!   on J and R a `Tree` row reads `static_rebuilds 1`, `members 1`, `evictions 0`, and
//!   `sleeper_rebuilds 0` (the structural receipts of gates G2 and G5).
//! * `--csv`: one row per step. Always `step, wall_ns, manifolds, pairs, top_y, awake`; armed also
//!   `void, colors, wide_colors, waves, g_ns, u_ns, r_ns, sys_sum_ns, disp_lane, worker_lane_max`
//!   and, per system and per physics zone, `<name>_ns` and `<name>_n` (counters: `<name>` is the
//!   value). `awake` is blank when sleeping is off.
//! * `pair_classes` (with `--contact-reuse`, or with contact reuse on in the row's config; `null`
//!   otherwise): the narrowphase's pair classes summed over the
//!   window (`Manifolds::pair_classes`, read after every step, untimed) — `reused`, `sep_hits`,
//!   `full`, `full_contacts`, `records_built`, `non_box`, `pairs` — and `h`, the share of the
//!   touching box pairs served from a record, `reused / (reused + full_contacts)`. A counter, not a
//!   time, so it is a result on a shared machine.
//! * exit code: 0 ok; 2 bad flags; 3 void (anti-vacuity, frozen-by, disarmed ring traffic,
//!   dropped samples, a reuse-on row with no reuse over steps [100, 500)); 4 `--expect-pose`
//!   mismatch; 101 panic.
//!
//! # Self-check (no `--scene`)
//!
//! `cargo test --all-targets` runs this binary with libtest's arguments, and `cargo bench` with
//! `--bench`. Without `--scene` it therefore ignores its arguments and runs a small self-check —
//! the `s16` scene, W = 1, 3 steps, disarmed — so the workspace test run exercises it in about a
//! second without timing anything. `--list` prints nothing.
//!
//! # Build (boyko)
//!
//! ```text
//! cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid
//! ```
//!
//! `[profile.parity]` (root `Cargo.toml`) inherits `release` — fat LTO, default codegen units, the
//! shipped profile (ruling W1). Every number names its profile. msvc host, no `RUSTFLAGS` (it would
//! replace the `x86-64-v3` baseline in `.cargo/config.toml`). The exe is run by path, never through
//! cargo. `BOYKO_PROFILE=shipping` builds the one disarmed `shipping` row (O6); `--arm-profiler` is
//! refused on a build whose tier folds the zones.
//!
//! # Build (Jolt)
//!
//! `benches/jolt_parity/pyramid_scene.patch` applies to Jolt v5.3.0 and v5.6.0 alike. It:
//!
//! * sets `mLinearDamping = mAngularDamping = 0` in `PyramidScene.h` (H3);
//! * adds `-no_pair_cache`: `PhysicsSettings::mUseBodyPairContactCache = false` (H10);
//! * adds `-allow_sleep`, threaded into `StartTest` through `PerformanceTestScene::SetAllowSleeping`
//!   (the scene hard-codes `mAllowSleeping = false` per body and only `-no_sleep` existed; O5). With
//!   it, `-f`'s per-frame CSV gains an `Active Bodies` column, the awake count O5 compares tails on;
//! * adds `-receipt` (untimed; H8): a counting `ContactListener` writes
//!   `receipt_<quality>_th<N>.csv` with, per frame, the manifolds and points it reported (added +
//!   persisted, cache hits included), the active body count and the top box's y;
//! * prints one `boyko-parity-patch` line naming the options in force.
//!
//! ```text
//! # WinLibs MinGW-w64 (g++, POSIX threads, UCRT) first on PATH: its bin directory holds
//! # c++.exe, gcc.exe and mingw32-make.exe.
//! cd D:/tmp/jolt/JoltPhysics            # the existing shallow clone at v5.3.0
//! git worktree add --detach D:/tmp/jolt/wt-v5.3.0-parity v5.3.0
//! git -C D:/tmp/jolt/wt-v5.3.0-parity apply <repo>/crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch
//! cmake -S D:/tmp/jolt/wt-v5.3.0-parity/Build -B D:/tmp/jolt/build-v5.3.0-dist -G "MinGW Makefiles" \
//!       -DCMAKE_BUILD_TYPE=Distribution -DTARGET_UNIT_TESTS=OFF -DTARGET_HELLO_WORLD=OFF \
//!       -DTARGET_SAMPLES=OFF -DTARGET_VIEWER=OFF
//! cmake --build D:/tmp/jolt/build-v5.3.0-dist --target PerformanceTest -j 6
//! # the exe imports libstdc++-6.dll and libwinpthread-1.dll (which imports libgcc_s_seh-1.dll):
//! # copy the three from the MinGW bin directory next to it, so a run by path needs no PATH.
//! # The same with -DCMAKE_BUILD_TYPE=Release into build-v5.3.0-release (the -p stage shares).
//! # v5.6.0: `git fetch --depth 1 origin tag v5.6.0` in the clone, a worktree at v5.6.0, the same
//! # patch and recipe into build-v5.6.0-dist. It needed no extra option on this host; its new
//! # compute options (JPH_USE_DX12 / _VK / _MTL / _CPU_COMPUTE) stay at their default ON and the
//! # driver's manifest records them (plan open question 2: recorded, never patched away).
//! ```
//!
//! The compiler installation is the one the 2026-09-10 binary's `CMakeCache.txt` names (g++ 16.1.0
//! on 2026-09-19), and the configure matches it option for option: Distribution, LTO
//! (`INTERPROCEDURAL_OPTIMIZATION`), and the AVX2/FMA/F16C/LZCNT/TZCNT set that is boyko's
//! `x86-64-v3`. `tools/physics_parity/driver.py manifest` records every binary's sha256, the
//! runtime DLLs beside it, its compiler, its CMake options and whether its source tree carries
//! exactly this patch (H6). Run: `PerformanceTest.exe -s=Pyramid -q=Discrete -t=W -f` in a fresh
//! working directory (it writes `per_frame_discrete_th<W>.csv` there). `-q=Discrete` only halves
//! the runtime: `PyramidScene::StartTest` never reads the motion quality (O4). `-no_pair_cache` is
//! NOT simulation-identical to the default run (its final hash differs after 5 steps): the cache
//! replays manifolds, so Δ_J prices a different trajectory as well as the skipped work.
//!
//! # What cannot be made identical
//!
//! The two solvers are different families: Jolt runs 10 velocity + 2 position iterations per step,
//! boyko 4 TGS-soft substeps of 1 + 2 sweeps — 12 sweeps each, different work per sweep. Each
//! engine runs its own defaults (H9); the report carries time per manifold-sweep beside the ratio.
//! Jolt keeps speculative contacts (0.02 m) and reduces manifolds, so the contact sets differ;
//! `-receipt` and this runner's `manifolds` column measure by how much (H8). Jolt's body-pair
//! cache stays on as an engine feature; `-no_pair_cache` prices it (H10).

// Alloc A/B: opt-in low-variance allocator, the same arm `bench_bevy_vs_boyko/benches/
// comparison_v2.rs` carries (Phase X.E). OFF by default, so a run measures the production system
// heap; `--features bench-alloc` swaps in mimalloc. Measured 2026-09-10 on this bench's Criterion
// predecessor: no effect at any W (docs/OPEN-QUESTIONS.md, "the system heap against mimalloc").
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use boyko_diag::lane::{LANE_COUNT, LANE_DISPATCHER, claim_lane};
use boyko_diag::profiling_abi::{ZoneHandle, zone_id};
use boyko_diag::sample::{Region, overflow, pending};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::profiling::{
    ArmOutcome, Profiler, ProfilerConfig, SYSTEM_ZONES_COMPILED, bind_world, fold_frame,
};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Res;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_macros::Resource;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::narrowphase::{NP_CHUNKS_PER_LANE, NP_MAX_CHUNKS, NP_MIN_PAIRS_PER_CHUNK};
use boyko_physics::broadphase_tree::{BroadphaseTree, TreeDiag};
use boyko_physics::plugin::{PhysicsStageKeys, add_physics_colored_solve, add_physics_systems};
use boyko_physics::profiling::{
    COUNTER_ZONE_COUNT, COUNTER_ZONES, PHYS_BP_ASSEMBLE, PHYS_BP_BUILD, PHYS_BP_MEMBERS,
    PHYS_BP_PAIRS, PHYS_BP_QUERIED, PHYS_BP_QUERY, PHYS_BP_REBUILDS, PHYS_BP_VERIFY,
    PHYS_COLOR_NARROW, PHYS_COLOR_WIDE, PHYS_GRAVITY, PHYS_INTEGRATE, PHYS_NP_AXIS_COMMIT,
    PHYS_NP_CHUNKS, PHYS_NP_COMPACT, PHYS_NP_DISPATCH, PHYS_NP_FULL, PHYS_NP_MANIFOLDS,
    PHYS_NP_PAIRS, PHYS_NP_POINTS, PHYS_NP_REUSED, PHYS_NP_SEP_HITS, PHYS_PASS_BIASED, PHYS_PASS_RELAX, PHYS_RESTITUTION, PHYS_SLEEP_BEGIN,
    PHYS_SLEEP_CLASSIFY, PHYS_SLEEP_END, PHYS_SLEEP_FREEZE, PHYS_SLEEP_HELD, PHYS_SLOTS_NARROW,
    PHYS_SLOTS_WIDE, PHYS_SOLVE_BUILD,
    PHYS_STORE, PHYS_WARM_APPLY, PHYS_WRITE_BACK, SPAN_ZONE_COUNT, SPAN_ZONES,
    WIDE_COLOR_MIN_SLOTS, ZONES_COMPILED,
};
use boyko_physics::resources::{
    BroadphaseKind, BroadphaseSelectMode, ConstraintGraph, ContactPairs, IslandSleep, Manifolds,
    PairClasses, PhysicsConfig, SleepSkip, SolverScratch,
};
use boyko_physics::sleep_sets::SleepSets;
use boyko_physics::solver::{ColoredSoftStepSolver, SoftStepSolver};

// ── Scene constants (Jolt `PyramidScene.h`, transcribed) ─────────────────────

/// `cBoxSize`: the pitch between neighbouring boxes in a layer.
const BOX_SIZE: f32 = 2.0;
/// `cHalfBoxSize`: every box's half-extent on every axis.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// `cBoxSeparation`: the `jolt` scene's default layer gap.
const JOLT_SEPARATION: f32 = 0.5;
/// `cPyramidHeight`: the number of layers, 1240 boxes.
const PYRAMID_HEIGHT: i32 = 15;
/// Dynamic bodies in the pyramid.
const PYRAMID_BODIES: usize = 1240;
/// Boxes in the `s16` tower.
const TOWER_BODIES: usize = 16;
/// The floor's half-extents, from Jolt's `BoxShape(Vec3(50, 1, 50))`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// Jolt's default friction (`BodyCreationSettings::mFriction`), the `jolt` scene's (H2).
const JOLT_FRICTION: f32 = 0.2;
/// The friction of the `rest` and `s16` scenes: `sleeping_pipeline.rs`'s and A7-R2's.
const REST_FRICTION: f32 = 0.5;
/// Jolt's `cDeltaTime`.
const DT: f32 = 1.0 / 60.0;
/// A unit-density cube of half-extent 1: m = 8.
const BOX_INV_MASS: f32 = 0.125;
/// Its inverse inertia, uniform on the diagonal: 1 / (m/12 · (2² + 2²)).
const BOX_INV_INERTIA: f32 = 0.1875;

// ── Runner constants ──────────────────────────────────────────────────────────

/// What the summary names the runner, so a receipt says which runner shape produced a row.
const RUNNER_ID: &str = "jolt_parity_pyramid fixed-window runner v1";
/// Exit code: bad flags.
const EXIT_USAGE: u8 = 2;
/// Exit code: the run is void.
const EXIT_VOID: u8 = 3;
/// Exit code: `--expect-pose` found different pose bytes.
const EXIT_POSE_MISMATCH: u8 = 4;
/// `RigidBody`'s thirteen `f32`s, as bytes: one body's share of the pose bytes.
const POSE_BYTES_PER_BODY: usize = 13 * 4;
/// FNV-1a 64 offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// The self-check's step count.
const SELF_CHECK_STEPS: usize = 3;
/// The steps a contact-reuse row must reuse a record in, or be void (L9 design, "Integration":
/// J's pile settles before step ~188, so a reuse-on row that reused nothing here measured nothing).
const REUSE_PROBE: (usize, usize) = (100, 500);

// ── Command line ──────────────────────────────────────────────────────────────

/// The scene a run spawns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SceneKind {
    /// Jolt's pyramid.
    Jolt,
    /// The exactly touching pyramid (`sleeping_pipeline.rs`'s `pyramid_sleeping_off`).
    Rest,
    /// A 16-box tower.
    S16,
}

impl SceneKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "jolt" => Some(Self::Jolt),
            "rest" => Some(Self::Rest),
            "s16" => Some(Self::S16),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Jolt => "jolt",
            Self::Rest => "rest",
            Self::S16 => "s16",
        }
    }

    fn default_gap(self) -> f32 {
        match self {
            Self::Jolt => JOLT_SEPARATION,
            Self::Rest | Self::S16 => 0.0,
        }
    }

    fn friction(self) -> f32 {
        match self {
            Self::Jolt => JOLT_FRICTION,
            Self::Rest | Self::S16 => REST_FRICTION,
        }
    }

    fn bodies(self) -> usize {
        match self {
            Self::Jolt | Self::Rest => PYRAMID_BODIES,
            Self::S16 => TOWER_BODIES,
        }
    }
}

/// Which solver pipeline is wired.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SolverKind {
    /// `add_physics_colored_solve`.
    Colored,
    /// `add_physics_systems::<SoftStepSolver>`.
    Reference,
}

/// Which configuration the run sets (see the module docs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CfgKind {
    /// cfg-A.
    A,
    /// cfg-As: cfg-A plus `simd_solve` (the L11 `J-As` row).
    As,
    /// cfg-B.
    B,
    /// The tree's `PhysicsConfig::default()`.
    Default,
}

/// L10's frozen-pair skip mode as `--sleep-skip` names it (rev 2.2 Δ1: Replay is retired).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SleepSkipArg {
    /// Every pair computed, as without L10.
    Off,
    /// Held islands' pairs skipped (L10 C3b).
    Sets,
}

impl SleepSkipArg {
    /// The configuration value.
    fn mode(self) -> SleepSkip {
        match self {
            Self::Off => SleepSkip::Off,
            Self::Sets => SleepSkip::Sets,
        }
    }
}

/// The parsed command line.
#[derive(Debug)]
struct Args {
    workers: usize,
    steps: usize,
    window: Option<(usize, usize)>,
    scene: SceneKind,
    gap: f32,
    solver: SolverKind,
    cfg: CfgKind,
    parallel_solve: bool,
    parallel_np: Option<bool>,
    contact_reuse: Option<bool>,
    reuse_distance: Option<f32>,
    broadphase: Option<BroadphaseKind>,
    /// `--sleeping [on|off]`; `None` leaves the --cfg's own value.
    sleeping: Option<bool>,
    sleep_skip: Option<SleepSkipArg>,
    threshold: Option<f32>,
    frozen_by: Option<usize>,
    arm_profiler: bool,
    canary_frac: Option<f64>,
    canary_ref_ns: Option<u64>,
    csv: Option<PathBuf>,
    pose_out: Option<PathBuf>,
    expect_pose: Option<PathBuf>,
    label: Option<String>,
    raw: Vec<String>,
}

/// What `main` does with the command line.
enum Mode {
    /// A measured run.
    Run(Box<Args>),
    /// No `--scene`: the self-check (or nothing, under `--list`).
    SelfCheck { list: bool },
}

/// Prints `msg` and the usage line to stderr; the caller exits with [`EXIT_USAGE`].
#[cold]
#[inline(never)]
fn usage_error(msg: &str) -> ExitCode {
    eprintln!("jolt_parity_pyramid: {msg}");
    eprintln!(
        "usage: jolt_parity_pyramid --scene jolt|rest|s16 [--workers W] [--steps N] [--window A..B] \
         [--gap G] [--solver colored|reference] [--cfg a|as|b|default] [--parallel-solve] \
         [--parallel-np on|off] [--contact-reuse on|off] [--reuse-distance D] \
         [--broadphase allpairs|tree|grid] [--sleeping [on|off]] [--sleep-skip off|sets] \
         [--threshold T] \
         [--frozen-by K] [--arm-profiler] [--canary-frac F --canary-ref-ns T] [--csv PATH] \
         [--pose-out PATH] [--expect-pose PATH] [--label TEXT]"
    );
    ExitCode::from(EXIT_USAGE)
}

fn parse_num<T: std::str::FromStr>(flag: &str, value: Option<String>) -> Result<T, String> {
    let value = value.ok_or_else(|| format!("{flag} needs a value"))?;
    value.parse().map_err(|_| format!("{flag}: cannot parse {value:?}"))
}

fn parse_window(value: Option<String>) -> Result<(usize, usize), String> {
    let value = value.ok_or("--window needs a value A..B")?;
    let (a, b) = value
        .split_once("..")
        .ok_or_else(|| format!("--window: expected A..B, got {value:?}"))?;
    let a = a.parse().map_err(|_| format!("--window: cannot parse {a:?}"))?;
    let b = b.parse().map_err(|_| format!("--window: cannot parse {b:?}"))?;
    Ok((a, b))
}

fn parse_args(raw: Vec<String>) -> Result<Mode, String> {
    if !raw.iter().any(|a| a == "--scene") {
        return Ok(Mode::SelfCheck { list: raw.iter().any(|a| a == "--list") });
    }
    let mut scene = None;
    let mut workers = 1usize;
    let mut steps = 500usize;
    let mut window = None;
    let mut gap = None;
    let mut solver = SolverKind::Colored;
    let mut cfg = CfgKind::Default;
    let mut parallel_solve = false;
    let mut parallel_np = None;
    let mut contact_reuse = None;
    let mut reuse_distance = None;
    let mut broadphase = None;
    let mut sleeping = None;
    let mut sleep_skip = None;
    let mut threshold = None;
    let mut frozen_by = None;
    let mut arm_profiler = false;
    let mut canary_frac = None;
    let mut canary_ref_ns = None;
    let mut csv = None;
    let mut pose_out = None;
    let mut expect_pose = None;
    let mut label = None;

    let mut it = raw.iter().cloned().peekable();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--bench" => {}
            "--scene" => {
                let v = it.next().ok_or("--scene needs a value")?;
                scene = Some(SceneKind::parse(&v).ok_or_else(|| format!("--scene: unknown {v:?}"))?);
            }
            "--workers" => workers = parse_num("--workers", it.next())?,
            "--steps" => steps = parse_num("--steps", it.next())?,
            "--window" => window = Some(parse_window(it.next())?),
            "--gap" => gap = Some(parse_num::<f32>("--gap", it.next())?),
            "--solver" => {
                solver = match it.next().as_deref() {
                    Some("colored") => SolverKind::Colored,
                    Some("reference") => SolverKind::Reference,
                    other => return Err(format!("--solver: expected colored|reference, got {other:?}")),
                }
            }
            "--cfg" => {
                cfg = match it.next().as_deref() {
                    Some("a") => CfgKind::A,
                    Some("as") => CfgKind::As,
                    Some("b") => CfgKind::B,
                    Some("default") => CfgKind::Default,
                    other => return Err(format!("--cfg: expected a|as|b|default, got {other:?}")),
                }
            }
            "--parallel-solve" => parallel_solve = true,
            "--parallel-np" => {
                parallel_np = match it.next().as_deref() {
                    Some("on") => Some(true),
                    Some("off") => Some(false),
                    other => return Err(format!("--parallel-np: expected on|off, got {other:?}")),
                }
            }
            "--contact-reuse" => {
                contact_reuse = match it.next().as_deref() {
                    Some("on") => Some(true),
                    Some("off") => Some(false),
                    other => return Err(format!("--contact-reuse: expected on|off, got {other:?}")),
                }
            }
            "--reuse-distance" => {
                reuse_distance = Some(parse_num::<f32>("--reuse-distance", it.next())?);
            }
            "--broadphase" => {
                broadphase = match it.next().as_deref() {
                    Some("allpairs") => Some(BroadphaseKind::AllPairs),
                    Some("tree") => Some(BroadphaseKind::Tree),
                    Some("grid") => Some(BroadphaseKind::Grid),
                    other => {
                        return Err(format!(
                            "--broadphase: expected allpairs|tree|grid, got {other:?}"
                        ));
                    }
                }
            }
            "--sleeping" => {
                sleeping = Some(match it.peek().map(String::as_str) {
                    Some("on") => {
                        it.next();
                        true
                    }
                    Some("off") => {
                        it.next();
                        false
                    }
                    // A bare `--sleeping` is `on`: the spelling of every recipe recorded before
                    // L10 C1a (the P0 queue, the L9 and L10 fixture READMEs).
                    _ => true,
                });
            }
            "--sleep-skip" => {
                sleep_skip = match it.next().as_deref() {
                    Some("off") => Some(SleepSkipArg::Off),
                    Some("sets") => Some(SleepSkipArg::Sets),
                    other => return Err(format!("--sleep-skip: expected off|sets, got {other:?}")),
                }
            }
            "--threshold" => threshold = Some(parse_num::<f32>("--threshold", it.next())?),
            "--frozen-by" => frozen_by = Some(parse_num::<usize>("--frozen-by", it.next())?),
            "--arm-profiler" => arm_profiler = true,
            "--canary-frac" => canary_frac = Some(parse_num::<f64>("--canary-frac", it.next())?),
            "--canary-ref-ns" => canary_ref_ns = Some(parse_num::<u64>("--canary-ref-ns", it.next())?),
            "--csv" => csv = Some(PathBuf::from(it.next().ok_or("--csv needs a path")?)),
            "--pose-out" => pose_out = Some(PathBuf::from(it.next().ok_or("--pose-out needs a path")?)),
            "--expect-pose" => {
                expect_pose = Some(PathBuf::from(it.next().ok_or("--expect-pose needs a path")?));
            }
            "--label" => label = Some(it.next().ok_or("--label needs a value")?),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    let scene = scene.ok_or("--scene needs a value")?;
    let args = Args {
        workers,
        steps,
        window,
        scene,
        gap: gap.unwrap_or_else(|| scene.default_gap()),
        solver,
        cfg,
        parallel_solve,
        parallel_np,
        contact_reuse,
        reuse_distance,
        broadphase,
        sleeping,
        sleep_skip,
        threshold,
        frozen_by,
        arm_profiler,
        canary_frac,
        canary_ref_ns,
        csv,
        pose_out,
        expect_pose,
        label,
        raw,
    };
    validate(&args)?;
    Ok(Mode::Run(Box::new(args)))
}

/// Refuses every combination a row could be mislabelled by.
fn validate(a: &Args) -> Result<(), String> {
    if a.workers == 0 || a.steps == 0 {
        return Err("--workers and --steps must be at least 1".into());
    }
    if let Some((lo, hi)) = a.window
        && !(lo < hi && hi <= a.steps)
    {
        return Err(format!("--window {lo}..{hi} is not a non-empty range inside 0..{}", a.steps));
    }
    if !a.gap.is_finite() || a.gap < 0.0 {
        return Err(format!("--gap {} must be finite and >= 0", a.gap));
    }
    if a.solver == SolverKind::Reference {
        if a.cfg != CfgKind::Default {
            return Err("--solver reference takes --cfg default only: cfg-A/As/B are colored".into());
        }
        if a.sleeping == Some(true) || a.parallel_solve || a.canary_frac.is_some() {
            return Err(
                "--solver reference has no sleeping, no parallel solve and no build_graph stage \
                 for the canary to precede"
                    .into(),
            );
        }
    }
    if a.parallel_solve && a.workers == 1 {
        return Err(
            "--parallel-solve at --workers 1 is J-P1, retired at L4: a one-worker pool solves \
             inline, so the row would report waves with no dispatch behind them; take ω₁ from \
             the zero-work spawn/join microbench"
                .into(),
        );
    }
    if a.threshold.is_some() && a.sleeping != Some(true) {
        return Err("--threshold needs --sleeping on".into());
    }
    if let Some(mode) = a.sleep_skip {
        // The mode is read only with sleeping on (a sleeping-off world runs Off whatever it
        // says), so a sleeping-off row would carry a mode it never ran.
        let sleeping = match (a.sleeping, a.cfg) {
            (Some(on), _) => on,
            (None, CfgKind::Default) => PhysicsConfig::default().sleeping,
            (None, _) => false,
        };
        if !sleeping || a.solver == SolverKind::Reference {
            return Err(format!(
                "--sleep-skip {mode:?} needs sleeping on and the colored solver: elsewhere the \
                 row would carry a mode it never runs"
            ));
        }
    }
    if let Some(d) = a.reuse_distance {
        if a.contact_reuse != Some(true) {
            return Err("--reuse-distance needs --contact-reuse on".into());
        }
        if !d.is_finite() || d < 0.0 {
            return Err(format!("--reuse-distance {d} must be finite and >= 0"));
        }
    }
    if let Some(k) = a.frozen_by {
        if a.sleeping != Some(true) {
            return Err("--frozen-by needs --sleeping on".into());
        }
        if k == 0 || k > a.steps {
            return Err(format!("--frozen-by {k} must name a step in 1..={}", a.steps));
        }
    }
    match (a.canary_frac, a.canary_ref_ns) {
        (None, None) => {}
        (Some(f), Some(t)) => {
            if !(f > 0.0 && f < 1.0) || t == 0 {
                return Err(format!("--canary-frac {f} must be in (0, 1) and --canary-ref-ns {t} > 0"));
            }
        }
        _ => return Err("--canary-frac and --canary-ref-ns go together".into()),
    }
    if a.arm_profiler && !(ZONES_COMPILED && SYSTEM_ZONES_COMPILED) {
        return Err(format!(
            "--arm-profiler on a build whose profile ({}) folds the zones: an armed run would \
             record nothing",
            boyko_diag::profile::PROFILE_NAME
        ));
    }
    if let Some(p) = &a.expect_pose
        && !p.is_file()
    {
        return Err(format!("--expect-pose: {} is not a file", p.display()));
    }
    Ok(())
}

// ── The scene ─────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD component as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` POD component borrowed for the returned
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes, read-only, which is
    // the layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box at rest into the `RigidBodyBundle` archetype. A dynamic one (`dynamic`) is
/// enabled as `Simulated`; the floor is not, which is how a static body is expressed here.
fn spawn_box(world: &mut EcsMaster, position: Vec3, friction: f32, dynamic: bool) -> Entity {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let (mass, half_extents) = if dynamic {
        (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                )),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        )
    } else {
        (
            RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction },
            FLOOR_HALF_EXTENTS,
        )
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("construction: the RigidBodyBundle archetype accepts the three columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
    e
}

/// Spawns the floor and the scene's dynamic bodies. Returns the dynamic bodies in spawn order;
/// the last one is the scene's top box.
fn spawn_scene(world: &mut EcsMaster, scene: SceneKind, gap: f32) -> Vec<Entity> {
    let friction = scene.friction();
    spawn_box(world, Vec3::new(0.0, -1.0, 0.0), friction, false);
    let mut boxes = Vec::with_capacity(scene.bodies());
    match scene {
        SceneKind::Jolt | SceneKind::Rest => {
            // Jolt's placement loop, index for index.
            for i in 0..PYRAMID_HEIGHT {
                let lo = i / 2;
                let hi = PYRAMID_HEIGHT - (i + 1) / 2;
                for j in lo..hi {
                    for k in lo..hi {
                        let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                        let position = Vec3::new(
                            -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                            1.0 + (BOX_SIZE + gap) * i as f32,
                            -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                        );
                        boxes.push(spawn_box(world, position, friction, true));
                    }
                }
            }
        }
        SceneKind::S16 => {
            for i in 0..TOWER_BODIES {
                let position = Vec3::new(0.0, 1.0 + (BOX_SIZE + gap) * i as f32, 0.0);
                boxes.push(spawn_box(world, position, friction, true));
            }
        }
    }
    boxes
}

/// The canary's spin, in ns.
#[derive(Resource)]
struct CanarySpin {
    ns: u64,
}

/// The canary system: a busy wait of [`CanarySpin::ns`] on whichever thread runs it.
fn parity_canary(spin: Res<CanarySpin>) {
    let target = Duration::from_nanos(spin.ns);
    let start = Instant::now();
    while start.elapsed() < target {
        std::hint::spin_loop();
    }
}

/// The measured world.
struct Rig {
    world: EcsMaster,
    physics: Schedule,
    boxes: Vec<Entity>,
}

/// Sets the knobs `args` names (module docs, "Configurations").
fn configure(cfg: &mut PhysicsConfig, args: &Args) {
    cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
    cfg.dt = DT;
    match args.cfg {
        CfgKind::A | CfgKind::As | CfgKind::B => {
            let parallel = args.parallel_solve || args.workers > 1;
            let b = args.cfg == CfgKind::B;
            cfg.parallel_solve = parallel;
            cfg.parallel_broadphase = parallel;
            cfg.parallel_narrowphase = parallel;
            cfg.broadphase_select = BroadphaseSelectMode::Manual;
            cfg.broadphase = if b { BroadphaseKind::Grid } else { BroadphaseKind::AllPairs };
            // cfg-As and cfg-B run the AVX2 cohort kernel; cfg-A the scalar oracle.
            cfg.simd_solve = args.cfg != CfgKind::A;
            // These configurations are pinned rows: sleeping is off unless the row asks.
            cfg.sleeping = args.sleeping.unwrap_or(false);
        }
        CfgKind::Default => {
            if args.parallel_solve {
                cfg.parallel_solve = true;
            }
            if let Some(sleeping) = args.sleeping {
                cfg.sleeping = sleeping;
            }
        }
    }
    if let Some(t) = args.threshold {
        cfg.sleep_threshold = t;
    }
    if let Some(mode) = args.sleep_skip {
        cfg.sleep_skip = mode.mode();
    }
    if let Some(np) = args.parallel_np {
        cfg.parallel_narrowphase = np;
    }
    if let Some(reuse) = args.contact_reuse {
        cfg.contact_reuse = reuse;
    }
    if let Some(d) = args.reuse_distance {
        cfg.contact_reuse_distance = d;
    }
    if let Some(kind) = args.broadphase {
        cfg.broadphase_select = BroadphaseSelectMode::Manual;
        cfg.broadphase = kind;
    }
}

/// Spawns the scene and wires the schedule. The one world of the process.
fn build(args: &Args, canary_ns: Option<u64>) -> Rig {
    let mut world = EcsMaster::new();
    let boxes = spawn_scene(&mut world, args.scene, args.gap);
    assert_eq!(
        boxes.len(),
        args.scene.bodies(),
        "construction: the {} scene must hold exactly {} dynamic bodies; a drift means the two \
         engines no longer run the same scene",
        args.scene.name(),
        args.scene.bodies()
    );
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(args.workers).build());
    let keys: PhysicsStageKeys = match args.solver {
        SolverKind::Colored => add_physics_colored_solve(&mut builder, &mut world),
        SolverKind::Reference => add_physics_systems::<SoftStepSolver>(&mut builder, &mut world),
    };
    if let Some(ns) = canary_ns {
        world.insert_resource(CanarySpin { ns });
        let canary = builder.add_system(parity_canary);
        // `PhysicsStageKeys` carries each stage's `SystemKey` inner index; the type is not
        // nameable here but its field is public, so the stage keys are copies of the canary's own
        // key with that index written in.
        let mut after = canary.key();
        after.0 = keys.narrowphase;
        let mut before = canary.key();
        before.0 = keys
            .build_graph
            .expect("invariant: the colored pipeline registers build_graph (validate() refuses the canary elsewhere)");
        canary.after(after).before(before);
    }
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    configure(world.resource_mut::<PhysicsConfig>(), args);
    let physics = builder.build(&mut world);
    Rig { world, physics, boxes }
}

// ── The profile ──────────────────────────────────────────────────────────────

/// Short name of a system's `type_name`: generics dropped, last path segment.
fn short_name(name: &str) -> String {
    let base = name.split('<').next().unwrap_or(name);
    base.rsplit("::").next().unwrap_or(base).to_owned()
}

/// Every zone the armed run diffs per step, in CSV column order.
struct ZoneTable {
    /// `(short name, id)` per system, in schedule order.
    systems: Vec<(String, u16)>,
    /// The solve system's index in `systems`, when the colored solve is wired.
    solve: Option<usize>,
    /// Ids of [`SPAN_ZONES`].
    spans: Vec<u16>,
    /// Ids of [`COUNTER_ZONES`].
    counters: Vec<u16>,
}

impl ZoneTable {
    fn new(physics: &Schedule) -> Self {
        let systems: Vec<(String, u16)> =
            physics.system_zones().map(|(name, id)| (short_name(name), id)).collect();
        let solve = systems.iter().position(|(n, _)| n == "physics_solve_colored");
        Self {
            systems,
            solve,
            spans: SPAN_ZONES.iter().map(|&h| zone_id(h)).collect(),
            counters: COUNTER_ZONES.iter().map(|&h| zone_id(h)).collect(),
        }
    }

    /// Every id, in column order: systems, spans, counters.
    fn ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.systems
            .iter()
            .map(|&(_, id)| id)
            .chain(self.spans.iter().copied())
            .chain(self.counters.iter().copied())
    }

    fn len(&self) -> usize {
        self.systems.len() + self.spans.len() + self.counters.len()
    }
}

/// Position of `handle` in [`SPAN_ZONES`].
fn span_index(handle: &ZoneHandle) -> usize {
    SPAN_ZONES
        .iter()
        .position(|&h| std::ptr::eq(h, handle))
        .expect("invariant: every physics span zone is listed in SPAN_ZONES")
}

/// Position of `handle` in [`COUNTER_ZONES`].
fn counter_index(handle: &ZoneHandle) -> usize {
    COUNTER_ZONES
        .iter()
        .position(|&h| std::ptr::eq(h, handle))
        .expect("invariant: every physics counter is listed in COUNTER_ZONES")
}

/// One step's structure, recomputed from the world after the step: what the solve was handed,
/// not what it built.
#[derive(Debug, Default, Clone, Copy)]
struct Shape {
    colors: u64,
    wide_colors: u64,
    narrow_colors: u64,
    wide_slots: u64,
    narrow_slots: u64,
    pairs: u64,
    manifolds: u64,
    points: u64,
    /// Gathered rows.
    rows: u64,
    /// The tree broadphase's `|S| + |Z|` after the step.
    bp_members: u64,
    /// Its admissions and compactions this step.
    bp_rebuilds: u64,
    /// Whether it classed a Wide or Excluded row this step (never, on these scenes).
    bp_kinds_moved: bool,
    /// Candidate pairs with a sphere on either side, counted here from the shapes.
    non_box: u64,
    /// The narrowphase's own pair classes (`Manifolds::pair_classes`, from its tags).
    classes: PairClasses,
    /// Rows L10's sleep-skip held after the step's broadphase (`SleepSets::stats`), `0` off the
    /// colored pipeline.
    held_rows: u64,
    /// Pairs L10's tree seam withheld from the stream (`SleepSets::stats`), `0` off the colored
    /// pipeline.
    withheld: u64,
    /// Dynamic rows awake this step (`IslandSleep::is_row_awake`), `0` with sleeping off.
    awake_dynamic: u64,
}

/// Whether the step took the colored solve's no-awake fast path (L10 C3a), derived from public
/// state, never from the solver: sleeping on, no dynamic row awake, and no manifold solved —
/// every stream manifold's island frozen, so the recomputed slots are zero. That step runs no
/// substep, no restitution pass and no freeze capture or restore.
fn fast_path(shape: &Shape, colored: bool, sleeping: bool) -> bool {
    colored && sleeping && shape.awake_dynamic == 0 && shape.wide_slots + shape.narrow_slots == 0
}

/// Recomputes this step's color classes and narrowphase output. A frozen island's manifolds are
/// excluded exactly as `build_columns` excludes them — the manifold's island is its dynamic side's
/// — using the per-step decision `IslandSleep::is_island_frozen` reports (`end_step` does not
/// change it, so it still describes the step that just ran).
fn step_shape(world: &EcsMaster, colored: bool, sleeping: bool, bp_prev: &mut TreeDiag) -> Shape {
    // The stream the graph colours: `ConstraintGraph::color` indexes it (L10 C2a).
    let manifolds = world.resource::<Manifolds>().solver_manifolds();
    let bp = world.resource::<BroadphaseTree>().diag();
    let sleep_stats = world.try_resource::<SleepSets>().map(SleepSets::stats).unwrap_or_default();
    let bodies = world.resource::<SolverScratch>().bodies();
    let is_box = |row: BodyIndex| matches!(bodies[row.0 as usize].shape, ColliderShape::Box { .. });
    let candidate_pairs = world.resource::<ContactPairs>().pairs();
    let mut shape = Shape {
        pairs: candidate_pairs.len() as u64,
        non_box: candidate_pairs.iter().filter(|&&(a, b)| !(is_box(a) && is_box(b))).count() as u64,
        classes: world.resource::<Manifolds>().pair_classes(),
        manifolds: manifolds.len() as u64,
        points: manifolds.iter().map(|m| u64::from(m.count)).sum(),
        rows: world.resource::<SolverScratch>().bodies_len() as u64,
        bp_members: bp.members,
        bp_rebuilds: (bp.static_rebuilds + bp.sleeper_rebuilds)
            - (bp_prev.static_rebuilds + bp_prev.sleeper_rebuilds),
        bp_kinds_moved: bp.wide_rows != bp_prev.wide_rows || bp.excluded_rows != bp_prev.excluded_rows,
        held_rows: u64::from(sleep_stats.held_rows),
        withheld: u64::from(sleep_stats.withheld_pairs),
        ..Shape::default()
    };
    *bp_prev = bp;
    if !colored {
        return shape;
    }
    let graph = world.resource::<ConstraintGraph>();
    let sleep = if sleeping { Some(world.resource::<IslandSleep>()) } else { None };
    shape.awake_dynamic = sleep.map_or(0, |s| {
        (0..bodies.len()).filter(|&r| bodies[r].inv_mass != 0.0 && s.is_row_awake(r)).count() as u64
    });
    shape.colors = u64::from(graph.n_colors());
    for c in 0..graph.n_colors() {
        let slots: u32 = graph
            .color(c)
            .iter()
            .map(|&mi| &manifolds[mi as usize])
            .filter(|m| {
                sleep.is_none_or(|s| {
                    let isl_a = graph.island_of(m.body_a.0);
                    let isl = if isl_a != ConstraintGraph::NO_ISLAND {
                        isl_a
                    } else {
                        graph.island_of(m.body_b.0)
                    };
                    isl == ConstraintGraph::NO_ISLAND || !s.is_island_frozen(isl)
                })
            })
            .map(|m| u32::from(m.count))
            .sum();
        if slots >= WIDE_COLOR_MIN_SLOTS {
            shape.wide_colors += 1;
            shape.wide_slots += u64::from(slots);
        } else {
            shape.narrow_colors += 1;
            shape.narrow_slots += u64::from(slots);
        }
    }
    shape
}

/// Σ over every lane and both regions of the samples pending and the samples refused: every push
/// that got past a gate moves one of the two.
fn ring_traffic() -> u64 {
    (0..LANE_COUNT)
        .flat_map(|lane| [Region::Engine, Region::User].map(|region| (lane, region)))
        .map(|(lane, region)| u64::from(pending(lane, region)) + overflow(lane, region))
        .sum()
}

/// The run's fixed structure: what the per-step expectations scale with.
#[derive(Clone, Copy)]
struct Structure {
    /// The colored solve is wired (its zones exist).
    colored: bool,
    /// Sleeping is on (the sleep zones run, frozen manifolds are skipped).
    sleeping: bool,
    /// `PhysicsConfig::substeps`.
    substeps: u64,
    /// `PhysicsConfig::relax_iterations`.
    relax: u64,
    /// `PhysicsConfig::parallel_narrowphase`.
    parallel_np: bool,
    /// The pool's worker count, `--workers`: the narrowphase's lanes.
    lanes: usize,
    /// `PhysicsConfig::broadphase`.
    broadphase: BroadphaseKind,
    /// `BroadphaseTree::brute_max_rows()`: with `Tree`, the tree path runs above it.
    brute_max_rows: u32,
    /// `PhysicsConfig::contact_reuse`.
    contact_reuse: bool,
    /// L10's step mode is `Sets`: the colored pipeline with sleeping on and
    /// `PhysicsConfig::sleep_skip == Sets`, so the broadphase runs the sleep-skip's prologue.
    sets: bool,
}

/// The narrowphase's chunk count for a step of `pairs` candidate pairs, or 0 when it runs the
/// serial loop: this runner's copy of `narrowphase/dispatch.rs`'s `chunk_count`, from the exported
/// constants, so a step whose zones disagree with it is void (ruling W4). It carries every term
/// the engine's carries — the flag, the `lanes < 2` term (lever ruling W1), the work term, the cap
/// and the two-chunk floor — so the two copies cannot agree by sharing an omission. The stage's
/// reserve ceiling (7.06M pairs) is not copied: no scene here comes near it.
fn expected_np_chunks(parallel_np: bool, pairs: u64, lanes: usize) -> u64 {
    if !parallel_np || lanes < 2 {
        return 0;
    }
    let pairs = usize::try_from(pairs).unwrap_or(usize::MAX);
    let chunks = (lanes * NP_CHUNKS_PER_LANE).min(pairs / NP_MIN_PAIRS_PER_CHUNK).min(NP_MAX_CHUNKS);
    if chunks < 2 { 0 } else { chunks as u64 }
}

/// Checks one armed step's per-zone sample counts and counter values against `shape`. Returns
/// the first mismatch.
fn check_step(
    zones: &ZoneTable,
    counts: &[u64],
    values: &[u64],
    shape: &Shape,
    st: Structure,
) -> Result<(), String> {
    let Structure {
        colored,
        sleeping,
        substeps,
        relax,
        parallel_np,
        lanes,
        broadphase,
        brute_max_rows,
        contact_reuse,
        sets,
    } = st;
    let n_sys = zones.systems.len();
    for (k, (name, _)) in zones.systems.iter().enumerate() {
        if counts[k] != 1 {
            return Err(format!("system `{name}` recorded {} spans, it runs once", counts[k]));
        }
    }
    let sweeps = substeps * (1 + relax);
    let c = u64::from(colored);
    let sl = u64::from(colored && sleeping);
    let l10 = u64::from(sets);
    // L10 C3a: on the no-awake fast path the substep loop, the restitution pass and the freeze
    // capture and restore do not run; the build, the store, the write-back and the sleep step do.
    let run = u64::from(!fast_path(shape, colored, sleeping));
    // The narrowphase collides the stream: the logical pairs less the ones L10's tree seam
    // withheld (design 06 §8).
    let stream = shape.pairs - shape.withheld;
    let np_chunks = expected_np_chunks(parallel_np, stream, lanes);
    let np = u64::from(np_chunks >= 2);
    // The tree path, derived from the configuration and the row count — never from the
    // implementation: `Tree` above `brute_max_rows` opens the four spans and emits the three
    // structural counters once; `AllPairs`, `Grid` and the Tree's brute path open none.
    let tree_path = broadphase == BroadphaseKind::Tree && shape.rows > u64::from(brute_max_rows);
    let tp = u64::from(tree_path);
    if tree_path && shape.bp_kinds_moved {
        return Err("the tree broadphase classed a Wide or Excluded row on a finite scene".to_owned());
    }
    let spans = &counts[n_sys..n_sys + SPAN_ZONES.len()];
    let expected: [(&ZoneHandle, u64); SPAN_ZONE_COUNT] = [
        (&PHYS_SOLVE_BUILD, c),
        (&PHYS_GRAVITY, c * run * substeps),
        (&PHYS_WARM_APPLY, c * run * substeps),
        (&PHYS_INTEGRATE, c * run * substeps),
        (&PHYS_PASS_BIASED, c * run * substeps),
        (&PHYS_PASS_RELAX, c * run * substeps * relax),
        (&PHYS_COLOR_WIDE, run * shape.wide_colors * sweeps),
        (&PHYS_COLOR_NARROW, run * shape.narrow_colors * sweeps),
        (&PHYS_RESTITUTION, c * run),
        (&PHYS_STORE, c),
        (&PHYS_WRITE_BACK, c),
        (&PHYS_SLEEP_BEGIN, sl),
        (&PHYS_SLEEP_FREEZE, 2 * sl * run),
        (&PHYS_SLEEP_END, sl),
        (&PHYS_SLEEP_CLASSIFY, l10),
        (&PHYS_NP_DISPATCH, np),
        (&PHYS_NP_COMPACT, np),
        (&PHYS_NP_AXIS_COMMIT, np),
        (&PHYS_BP_VERIFY, tp),
        (&PHYS_BP_BUILD, tp),
        (&PHYS_BP_QUERY, tp),
        (&PHYS_BP_ASSEMBLE, tp),
    ];
    for &(handle, want) in &expected {
        let got = spans[span_index(handle)];
        if got != want {
            return Err(format!("`{}` recorded {got} spans, the structure has {want}", handle.desc.name));
        }
    }
    let base = n_sys + SPAN_ZONES.len();
    // L9's pair classes close over the step's pairs (ruling W4): the tags' own non-box count is
    // the shapes', the five classes (L10's held skips among them, design 06 §8) sum to the
    // stream, and a row without reuse reuses nothing.
    let cl = shape.classes;
    if (cl.pairs, cl.non_box) != (stream, shape.non_box)
        || cl.full + cl.reused + cl.sep_hits + cl.non_box + cl.held_skipped != stream
        || (!contact_reuse && cl.reused != 0)
    {
        return Err(format!(
            "the pair classes {cl:?} do not close over {stream} stream pairs ({} non-box, \
             contact_reuse {contact_reuse})",
            shape.non_box
        ));
    }
    // On a tree-path step every row is queried or a member (`q + m == N`: these scenes have no
    // Wide or Excluded row), so the queried value is `N − members`.
    let expected_counters: [(&ZoneHandle, u64, u64); COUNTER_ZONE_COUNT] = [
        (&PHYS_SLOTS_WIDE, c, shape.wide_slots),
        (&PHYS_SLOTS_NARROW, c, shape.narrow_slots),
        (&PHYS_NP_PAIRS, 1, stream),
        (&PHYS_NP_MANIFOLDS, 1, shape.manifolds),
        (&PHYS_NP_POINTS, 1, shape.points),
        (&PHYS_BP_PAIRS, 1, shape.pairs),
        (&PHYS_NP_CHUNKS, 1, np_chunks),
        (&PHYS_BP_QUERIED, tp, shape.rows - shape.bp_members),
        (&PHYS_BP_MEMBERS, tp, shape.bp_members),
        (&PHYS_BP_REBUILDS, tp, shape.bp_rebuilds),
        (&PHYS_NP_REUSED, 1, cl.reused),
        (&PHYS_NP_SEP_HITS, 1, cl.sep_hits),
        (&PHYS_NP_FULL, 1, cl.full),
        (&PHYS_SLEEP_HELD, l10, shape.held_rows),
    ];
    for &(handle, want_n, want_v) in &expected_counters {
        let k = base + counter_index(handle);
        let (n, v) = (counts[k], values[k]);
        if (n, v) != (want_n, want_v * want_n) {
            return Err(format!(
                "counter `{}` recorded (samples, value) = ({n}, {v}), the step has ({want_n}, {})",
                handle.desc.name,
                want_v * want_n
            ));
        }
    }
    Ok(())
}

// ── Recording ─────────────────────────────────────────────────────────────────

/// What every step records, armed or not. All read after the step's `Instant` pair closed.
struct StepRow {
    wall_ns: u64,
    manifolds: u64,
    pairs: u64,
    top_y: f32,
    awake: Option<u64>,
}

/// What an armed step adds.
struct ArmedRow {
    void: bool,
    shape: Shape,
    waves: u64,
    g_ns: i64,
    u_ns: Option<i64>,
    r_ns: i64,
    sys_sum_ns: u64,
    disp_lane: u64,
    worker_lane_max: u64,
}

/// FNV-1a 64 over `bytes`.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(FNV_PRIME))
}

/// Every dynamic body's full `RigidBody`, as little-endian `f32` bits, in spawn order.
fn pose_bytes(world: &EcsMaster, boxes: &[Entity]) -> Vec<u8> {
    let mut out = Vec::with_capacity(boxes.len() * POSE_BYTES_PER_BODY);
    for &e in boxes {
        let b = world.get_component::<RigidBody>(e).expect("invariant: a spawned body is live");
        let fields = [
            b.position.x,
            b.position.y,
            b.position.z,
            b.linear_velocity.x,
            b.linear_velocity.y,
            b.linear_velocity.z,
            b.rotation.x,
            b.rotation.y,
            b.rotation.z,
            b.rotation.w,
            b.angular_velocity.x,
            b.angular_velocity.y,
            b.angular_velocity.z,
        ];
        for f in fields {
            out.extend_from_slice(&f.to_bits().to_le_bytes());
        }
    }
    out
}

/// Dynamic rows awake on the last step, or `None` when sleeping is off.
fn awake_rows(world: &EcsMaster, sleeping: bool) -> Option<u64> {
    if !sleeping {
        return None;
    }
    let sleep = world.resource::<IslandSleep>();
    let rows = world.resource::<SolverScratch>().bodies();
    Some(
        (0..rows.len())
            .filter(|&r| rows[r].inv_mass != 0.0 && sleep.is_row_awake(r))
            .count() as u64,
    )
}

/// A JSON string literal.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number, or `null` for a non-finite value.
fn json_f64(x: f64) -> String {
    if x.is_finite() { format!("{x}") } else { "null".to_owned() }
}

// ── The run ───────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(raw) {
        Err(msg) => usage_error(&msg),
        Ok(Mode::SelfCheck { list: true }) => ExitCode::SUCCESS,
        Ok(Mode::SelfCheck { list: false }) => self_check(),
        Ok(Mode::Run(args)) => run(&args),
    }
}

/// The no-`--scene` path (module docs, "Self-check").
fn self_check() -> ExitCode {
    let args = Args {
        workers: 1,
        steps: SELF_CHECK_STEPS,
        window: None,
        scene: SceneKind::S16,
        gap: SceneKind::S16.default_gap(),
        solver: SolverKind::Colored,
        cfg: CfgKind::A,
        parallel_solve: false,
        parallel_np: None,
        contact_reuse: None,
        reuse_distance: None,
        broadphase: None,
        sleeping: None,
        sleep_skip: None,
        threshold: None,
        frozen_by: None,
        arm_profiler: false,
        canary_frac: None,
        canary_ref_ns: None,
        csv: None,
        pose_out: None,
        expect_pose: None,
        label: Some("self-check".to_owned()),
        raw: Vec::new(),
    };
    println!(
        "jolt_parity_pyramid: no --scene, so this is the self-check ({SELF_CHECK_STEPS} steps of \
         s16, W=1, disarmed); pass --scene to run a row"
    );
    run(&args)
}

fn run(args: &Args) -> ExitCode {
    let armed = args.arm_profiler;
    let canary_ns = args
        .canary_frac
        .zip(args.canary_ref_ns)
        .map(|(f, t)| (f * t as f64).round() as u64);
    let window = args.window.unwrap_or((0, args.steps));

    // The fold's own span is pushed from this thread, and a push from a thread without a lane is
    // an `Unclaimed` drop.
    let lane_claimed = claim_lane().is_some();
    if armed && !lane_claimed {
        eprintln!("jolt_parity_pyramid: no spare diagnostics lane for the driving thread");
        return ExitCode::from(EXIT_VOID);
    }

    let traffic_before = ring_traffic();
    let mut rig = build(args, canary_ns);
    let colored = args.solver == SolverKind::Colored;
    let (substeps, relax, sleeping, sleep_skip, parallel_np, contact_reuse, config_json) = {
        let cfg = rig.world.resource::<PhysicsConfig>();
        let tree_brute_max_rows = rig.world.resource::<BroadphaseTree>().brute_max_rows();
        let json = format!(
            "{{\"substeps\":{},\"relax_iterations\":{},\"broadphase\":{},\"broadphase_select\":{},\
             \"tree_brute_max_rows\":{tree_brute_max_rows},\
             \"simd\":{},\"simd_solve\":{},\"parallel_solve\":{},\"parallel_broadphase\":{},\
             \"parallel_narrowphase\":{},\"contact_reuse\":{},\"contact_reuse_distance\":{},\
             \"sleeping\":{},\"sleep_skip\":{},\"sleep_threshold\":{},\"sleep_frames\":{},\
             \"colored\":{},\"contact_hertz\":{},\"contact_damping\":{}}}",
            cfg.substeps,
            cfg.relax_iterations,
            json_str(&format!("{:?}", cfg.broadphase)),
            json_str(&format!("{:?}", cfg.broadphase_select)),
            cfg.simd,
            cfg.simd_solve,
            cfg.parallel_solve,
            cfg.parallel_broadphase,
            cfg.parallel_narrowphase,
            cfg.contact_reuse,
            json_f64(f64::from(cfg.contact_reuse_distance)),
            cfg.sleeping,
            json_str(&format!("{:?}", cfg.sleep_skip)),
            json_f64(f64::from(cfg.sleep_threshold)),
            cfg.sleep_frames,
            cfg.colored,
            json_f64(f64::from(cfg.contact_hertz)),
            json_f64(f64::from(cfg.contact_damping)),
        );
        (
            u64::from(cfg.substeps),
            u64::from(cfg.relax_iterations),
            cfg.sleeping,
            cfg.sleep_skip,
            cfg.parallel_narrowphase,
            cfg.contact_reuse,
            json,
        )
    };

    let zones = if armed {
        bind_world(rig.world.world_id().get())
            .expect("invariant: the runner builds exactly one world per process");
        rig.world.insert_resource(Profiler::new());
        let outcome = rig.world.resource_mut::<Profiler>().arm(ProfilerConfig::default());
        assert_eq!(outcome, ArmOutcome::Armed, "invariant: the process's first arm");
        Some(ZoneTable::new(&rig.physics))
    } else {
        None
    };
    let (broadphase, brute_max_rows) = (
        rig.world.resource::<PhysicsConfig>().broadphase,
        rig.world.resource::<BroadphaseTree>().brute_max_rows(),
    );
    let structure = Structure {
        colored,
        sleeping,
        substeps,
        relax,
        parallel_np,
        lanes: args.workers,
        broadphase,
        brute_max_rows,
        contact_reuse,
        sets: colored && sleeping && sleep_skip == SleepSkip::Sets,
    };
    let mut bp_prev = rig.world.resource::<BroadphaseTree>().diag();
    let n_cols = zones.as_ref().map_or(0, ZoneTable::len);
    let ids: Vec<u16> = zones.as_ref().map_or_else(Vec::new, |z| z.ids().collect());
    let tpn = if armed { boyko_diag::clock::ticks_per_ns() } else { 0.0 };

    let mut rows: Vec<StepRow> = Vec::with_capacity(args.steps);
    let mut armed_rows: Vec<ArmedRow> = Vec::with_capacity(if armed { args.steps } else { 0 });
    // Per step and column: (samples, Σ value). Spans are ticks, counters their value.
    let mut deltas: Vec<(u64, u64)> = Vec::with_capacity(args.steps * n_cols);
    let mut before: Vec<(u64, u64)> = vec![(0, 0); n_cols];
    let mut counts = vec![0u64; n_cols];
    let mut values = vec![0u64; n_cols];
    let mut void_steps = 0usize;
    let mut first_void: Option<String> = None;
    let mut solve_on_dispatcher_steps = 0usize;
    // L10 C3a: armed steps whose recomputed structure is the no-awake fast path.
    let mut fast_steps_derived = 0u64;
    let mut first_frozen_step: Option<usize> = None;
    // L10 C3b: the held set's witness (untimed): steps with a held row, and the most rows held.
    let (mut held_steps, mut held_rows_max) = (0u64, 0u32);
    // L10 C3c: the tree seam's witness (untimed): steps with a withheld pair, the most pairs
    // withheld, and the first step with every dynamic row held.
    let (mut withheld_steps, mut withheld_max, mut first_all_held) = (0u64, 0u32, None::<usize>);
    // L9: the pair classes, per step. Read on every row whose EFFECTIVE config has reuse on, since
    // the void probe after the run reads them there whether or not the row named the flag, and on
    // every row that names `--contact-reuse` (the summary's `pair_classes`). A reuse-off row that
    // does not name the flag walks no tags.
    let read_classes = contact_reuse || args.contact_reuse.is_some();
    let mut classes: Vec<PairClasses> =
        Vec::with_capacity(if read_classes { args.steps } else { 0 });
    let top = *rig.boxes.last().expect("invariant: every scene spawns dynamic bodies");

    for step in 0..args.steps {
        let t0 = Instant::now();
        rig.physics.run(&mut rig.world);
        let wall = t0.elapsed();
        // ── Untimed from here. ──

        let (disp_lane, worker_lane_max) = if armed {
            let workers = u16::try_from(args.workers).unwrap_or(u16::MAX).min(LANE_DISPATCHER);
            (
                u64::from(pending(LANE_DISPATCHER, Region::Engine)),
                (0..workers).map(|l| u64::from(pending(l, Region::Engine))).max().unwrap_or(0),
            )
        } else {
            (0, 0)
        };
        fold_frame(&mut rig.world);

        let manifolds = rig.world.resource::<Manifolds>().manifolds().len() as u64;
        let pairs = rig.world.resource::<ContactPairs>().pairs().len() as u64;
        let top_y = rig
            .world
            .get_component::<RigidBody>(top)
            .expect("invariant: the top box is live")
            .position
            .y;
        let awake = awake_rows(&rig.world, colored && sleeping);
        if awake == Some(0) && first_frozen_step.is_none() {
            first_frozen_step = Some(step + 1);
        }
        if let Some(k) = args.frozen_by
            && step + 1 == k
            && awake != Some(0)
        {
            void_steps += 1;
            first_void.get_or_insert_with(|| {
                format!("step {step}: --frozen-by {k}: {awake:?} dynamic rows still awake")
            });
        }
        rows.push(StepRow { wall_ns: wall.as_nanos() as u64, manifolds, pairs, top_y, awake });
        if let Some(sets) = rig.world.try_resource::<SleepSets>() {
            let stats = sets.stats();
            let held = stats.held_rows;
            held_steps += u64::from(held > 0);
            held_rows_max = held_rows_max.max(held);
            withheld_steps += u64::from(stats.withheld_pairs > 0);
            withheld_max = withheld_max.max(stats.withheld_pairs);
            if first_all_held.is_none() && held as usize == rig.boxes.len() {
                first_all_held = Some(step + 1);
            }
        }
        if read_classes {
            classes.push(rig.world.resource::<Manifolds>().pair_classes());
        }

        if let Some(zones) = &zones {
            let profiler = rig.world.resource::<Profiler>();
            for (k, id) in ids.iter().enumerate() {
                let acc = profiler
                    .lifetime(*id)
                    .expect("invariant: every zone id of this process is inside the armed geometry");
                counts[k] = acc.count - before[k].0;
                values[k] = acc.total.wrapping_sub(before[k].1);
                before[k] = (acc.count, acc.total);
                deltas.push((counts[k], values[k]));
            }
            let shape = step_shape(&rig.world, colored, sleeping, &mut bp_prev);
            let fast = fast_path(&shape, colored, sleeping);
            fast_steps_derived += u64::from(fast);
            let verdict = check_step(zones, &counts, &values, &shape, structure);
            let void = verdict.is_err();
            if let Err(why) = verdict {
                void_steps += 1;
                first_void.get_or_insert_with(|| format!("step {step}: {why}"));
            }

            let ns = |ticks: u64| ticks as f64 / tpn;
            let n_sys = zones.systems.len();
            let span = |h: &ZoneHandle| n_sys + span_index(h);
            let sys_sum: f64 = (0..n_sys).map(|k| ns(values[k])).sum();
            let in_solve: f64 = [
                &PHYS_SOLVE_BUILD,
                &PHYS_GRAVITY,
                &PHYS_WARM_APPLY,
                &PHYS_INTEGRATE,
                &PHYS_PASS_BIASED,
                &PHYS_PASS_RELAX,
                &PHYS_RESTITUTION,
                &PHYS_STORE,
                &PHYS_WRITE_BACK,
                &PHYS_SLEEP_BEGIN,
                &PHYS_SLEEP_FREEZE,
                &PHYS_SLEEP_END,
            ]
            .iter()
            .map(|h| ns(values[span(h)]))
            .sum();
            let passes = ns(values[span(&PHYS_PASS_BIASED)]) + ns(values[span(&PHYS_PASS_RELAX)]);
            let colors = ns(values[span(&PHYS_COLOR_WIDE)]) + ns(values[span(&PHYS_COLOR_NARROW)]);
            // The solve's own samples: every span but the narrowphase's, which the narrowphase
            // system pushes from whichever thread runs it, and L10's classification, which the
            // broadphase system pushes likewise.
            let np_spans =
                [&PHYS_NP_DISPATCH, &PHYS_NP_COMPACT, &PHYS_NP_AXIS_COMMIT, &PHYS_SLEEP_CLASSIFY]
                    .map(span);
            let solve_samples: u64 = (n_sys..n_sys + SPAN_ZONES.len())
                .filter(|k| !np_spans.contains(k))
                .map(|k| counts[k])
                .sum::<u64>()
                + counts[n_sys + SPAN_ZONES.len() + counter_index(&PHYS_SLOTS_WIDE)]
                + counts[n_sys + SPAN_ZONES.len() + counter_index(&PHYS_SLOTS_NARROW)];
            // A fast-path step (L10 C3a) pushes too few solve samples for the lane comparison to
            // say where the solve ran, so the witness reads the full-path steps only.
            if !fast && solve_samples > 0 && disp_lane >= solve_samples {
                solve_on_dispatcher_steps += 1;
            }
            armed_rows.push(ArmedRow {
                void,
                shape,
                waves: counts[span(&PHYS_COLOR_WIDE)],
                g_ns: (wall.as_nanos() as f64 - sys_sum).round() as i64,
                u_ns: zones.solve.map(|s| (ns(values[s]) - in_solve).round() as i64),
                r_ns: (passes - colors).round() as i64,
                sys_sum_ns: sys_sum.round() as u64,
                disp_lane,
                worker_lane_max,
            });
        }
    }

    // ── After the run. ──
    // L10 C3a: the solver's own count of no-awake fast-path steps, and, armed, the same count
    // derived from public state step by step: a solver that took the path on a step the
    // structure does not allow it on (or skipped it where it does) voids the row.
    let fast_steps_engine =
        colored.then(|| rig.world.resource::<ColoredSoftStepSolver>().fast_path_steps());
    if armed
        && let Some(engine) = fast_steps_engine
        && engine != fast_steps_derived
    {
        void_steps += 1;
        first_void.get_or_insert_with(|| {
            format!(
                "the solver took its no-awake fast path on {engine} steps, the structure allows \
                 {fast_steps_derived}"
            )
        });
    }
    let drops = rig.world.contains_resource::<Profiler>().then(|| rig.world.resource::<Profiler>().drops());
    if let Some(d) = drops
        && d.total() != 0
    {
        void_steps += 1;
        first_void.get_or_insert_with(|| format!("the store dropped samples: {d:?}"));
    }
    let traffic = ring_traffic().wrapping_sub(traffic_before);
    if !armed && traffic != 0 {
        void_steps += 1;
        first_void.get_or_insert_with(|| format!("the disarmed run pushed {traffic} samples"));
    }
    // L9 (design, "Integration"): a reuse-on row that reused nothing over steps [100, 500) is void.
    // It tests the effective config, which is also what `read_classes` tests, so `classes` holds
    // one entry per step on every row it indexes, the flag named or not.
    let reuse_probe = REUSE_PROBE.0..REUSE_PROBE.1.min(args.steps);
    if contact_reuse
        && !reuse_probe.is_empty()
        && classes[reuse_probe.clone()].iter().all(|c| c.reused == 0)
    {
        void_steps += 1;
        first_void.get_or_insert_with(|| {
            format!("contact reuse is on and no pair reused its record over steps {reuse_probe:?}")
        });
    }
    let classes_json = if classes.is_empty() {
        "null".to_owned()
    } else {
        let sum = classes[window.0..window.1].iter().fold(PairClasses::default(), |s, c| PairClasses {
            pairs: s.pairs + c.pairs,
            held_skipped: s.held_skipped + c.held_skipped,
            non_box: s.non_box + c.non_box,
            sep_hits: s.sep_hits + c.sep_hits,
            reused: s.reused + c.reused,
            full: s.full + c.full,
            full_contacts: s.full_contacts + c.full_contacts,
            records_built: s.records_built + c.records_built,
        });
        let touching = sum.reused + sum.full_contacts;
        let h = if touching == 0 { f64::NAN } else { sum.reused as f64 / touching as f64 };
        format!(
            "{{\"window\":[{},{}],\"pairs\":{},\"non_box\":{},\"sep_hits\":{},\"reused\":{},\
             \"full\":{},\"full_contacts\":{},\"records_built\":{},\"h\":{}}}",
            window.0,
            window.1,
            sum.pairs,
            sum.non_box,
            sum.sep_hits,
            sum.reused,
            sum.full,
            sum.full_contacts,
            sum.records_built,
            json_f64(h),
        )
    };

    let pose = pose_bytes(&rig.world, &rig.boxes);
    let pose_hash = fnv1a64(&pose);
    let mut exit = if void_steps > 0 { EXIT_VOID } else { 0 };
    if let Some(path) = &args.pose_out
        && let Err(e) = std::fs::write(path, &pose)
    {
        eprintln!("jolt_parity_pyramid: writing {}: {e}", path.display());
        exit = EXIT_USAGE;
    }
    let pose_verdict = match &args.expect_pose {
        None => "none".to_owned(),
        Some(path) => match std::fs::read(path) {
            Err(e) => {
                exit = EXIT_USAGE;
                format!("unreadable: {e}")
            }
            Ok(want) if want == pose => "match".to_owned(),
            Ok(want) => {
                exit = exit.max(EXIT_POSE_MISMATCH);
                let first = want
                    .chunks(POSE_BYTES_PER_BODY)
                    .zip(pose.chunks(POSE_BYTES_PER_BODY))
                    .position(|(a, b)| a != b);
                format!(
                    "mismatch: {} vs {} bytes, first differing body {first:?}",
                    want.len(),
                    pose.len()
                )
            }
        },
    };

    if let Some(path) = &args.csv
        && let Err(e) = std::fs::write(path, csv_text(&rows, &armed_rows, zones.as_ref(), &deltas, tpn))
    {
        eprintln!("jolt_parity_pyramid: writing {}: {e}", path.display());
        exit = EXIT_USAGE;
    }

    // The summary.
    let win = &rows[window.0..window.1];
    let win_ns: u64 = win.iter().map(|r| r.wall_ns).sum();
    let win_mean = win_ns as f64 / win.len() as f64;
    let steps_per_s = win.len() as f64 / (win_ns as f64 * 1e-9);
    let last = rows.last().expect("invariant: --steps >= 1");
    let waves_total: u64 = armed_rows.iter().map(|r| r.waves).sum();
    let disp_max = armed_rows.iter().map(|r| r.disp_lane).max().unwrap_or(0);
    let awake_after = args.frozen_by.map(|k| rows[k - 1..].iter().filter_map(|r| r.awake).max().unwrap_or(0));
    // The tree broadphase's structural receipt (module docs, "Output"); all zero off the Tree.
    let bp = rig.world.resource::<BroadphaseTree>().diag();
    let bp_json = format!(
        "{{\"static_rebuilds\":{},\"sleeper_rebuilds\":{},\"evictions\":{},\"translations\":{},\
         \"patches\":{},\"hint_candidates\":{},\"wide_rows\":{},\"excluded_rows\":{},\
         \"locator_resets\":{},\"members\":{}}}",
        bp.static_rebuilds,
        bp.sleeper_rebuilds,
        bp.evictions,
        bp.translations,
        bp.patches,
        bp.hint_candidates,
        bp.wide_rows,
        bp.excluded_rows,
        bp.locator_resets,
        bp.members,
    );

    println!(
        "jolt_parity_pyramid: scene {} gap {} friction {} bodies {} workers {} solver {:?} cfg {:?} \
         profile {} armed {armed}",
        args.scene.name(),
        args.gap,
        args.scene.friction(),
        rig.boxes.len(),
        args.workers,
        args.solver,
        args.cfg,
        boyko_diag::profile::PROFILE_NAME,
    );
    println!(
        "steps {} window {}..{} final manifolds {} pairs {} top_y {} pose_hash {pose_hash:#018x}",
        args.steps, window.0, window.1, last.manifolds, last.pairs, last.top_y
    );
    if armed {
        println!(
            "profile: void steps {void_steps}, waves {waves_total}, solve on dispatcher {solve_on_dispatcher_steps} of {} steps",
            armed_rows.len()
        );
    }
    if let Some(engine) = fast_steps_engine {
        println!(
            "no-awake fast path (L10 C3a): {engine} steps{}",
            if armed { format!(", {fast_steps_derived} derived from the structure") } else { String::new() }
        );
    }
    if let Some(sets) = rig.world.try_resource::<SleepSets>() {
        println!(
            "sleep-skip (L10 C3b): mode {:?}, {held_steps} steps with a held row, at most \
             {held_rows_max} rows held, last step {:?}, rules {:?}",
            sets.step_mode(),
            sets.stats(),
            sets.rule_counts()
        );
        println!(
            "tree seam (L10 C3c): {withheld_steps} steps with a withheld pair, at most \
             {withheld_max} withheld, every dynamic row held from step {first_all_held:?}, \
             sleepers at the end {}",
            rig.world.resource::<BroadphaseTree>().sleeper_members()
        );
    }
    if let Some(why) = &first_void {
        println!("VOID: {why}");
    }

    let mut s = String::with_capacity(2048);
    let _ = write!(
        s,
        "{{\"runner\":{},\"label\":{},\"args\":[{}],\"profile_name\":{},\"zones_compiled\":{},\
         \"system_zones_compiled\":{},\"debug_assertions\":{},\"target_env\":{},\
         \"scene\":{},\"gap\":{},\"friction\":{},\"bodies\":{},\"workers\":{},\"solver\":{},\
         \"cfg\":{},\"config\":{config_json},\"armed\":{armed},\"canary_ns\":{},\
         \"steps\":{},\"window\":[{},{}],\"window_mean_ns\":{},\"window_steps_per_s\":{},\
         \"pose_hash\":\"{pose_hash:#018x}\",\"pose_bytes\":{},\"expect_pose\":{},\
         \"final_manifolds\":{},\"final_pairs\":{},\"final_top_y\":{},\
         \"void_steps\":{void_steps},\"first_void\":{},\"drops_total\":{},\
         \"disarmed_ring_traffic\":{},\"ticks_per_ns\":{},\"waves_total\":{waves_total},\
         \"first_frozen_step\":{},\"frozen_by\":{},\"awake_max_from_frozen_by\":{},\
         \"broadphase_tree\":{bp_json},\"pair_classes\":{classes_json},\
         \"threads\":{{\"pool_workers\":{},\"dispatcher\":1,\"solve_on_dispatcher_steps\":\
         {solve_on_dispatcher_steps},\"armed_steps\":{},\"dispatcher_lane_samples_max\":{disp_max}}}}}",
        json_str(RUNNER_ID),
        args.label.as_deref().map_or_else(|| "null".to_owned(), json_str),
        args.raw.iter().map(|a| json_str(a)).collect::<Vec<_>>().join(","),
        json_str(boyko_diag::profile::PROFILE_NAME),
        ZONES_COMPILED,
        SYSTEM_ZONES_COMPILED,
        cfg!(debug_assertions),
        json_str(if cfg!(target_env = "msvc") { "msvc" } else if cfg!(target_env = "gnu") { "gnu" } else { "other" }),
        json_str(args.scene.name()),
        json_f64(f64::from(args.gap)),
        json_f64(f64::from(args.scene.friction())),
        rig.boxes.len(),
        args.workers,
        json_str(match args.solver {
            SolverKind::Colored => "colored",
            SolverKind::Reference => "reference",
        }),
        json_str(match args.cfg {
            CfgKind::A => "a",
            CfgKind::As => "as",
            CfgKind::B => "b",
            CfgKind::Default => "default",
        }),
        canary_ns.map_or_else(|| "null".to_owned(), |n| n.to_string()),
        args.steps,
        window.0,
        window.1,
        json_f64(win_mean),
        json_f64(steps_per_s),
        pose.len(),
        json_str(&pose_verdict),
        last.manifolds,
        last.pairs,
        json_f64(f64::from(last.top_y)),
        first_void.as_deref().map_or_else(|| "null".to_owned(), json_str),
        drops.map_or_else(|| "null".to_owned(), |d| d.total().to_string()),
        if armed { "null".to_owned() } else { traffic.to_string() },
        json_f64(tpn),
        first_frozen_step.map_or_else(|| "null".to_owned(), |k| k.to_string()),
        args.frozen_by.map_or_else(|| "null".to_owned(), |k| k.to_string()),
        awake_after.map_or_else(|| "null".to_owned(), |k| k.to_string()),
        args.workers,
        armed_rows.len(),
    );
    println!("SUMMARY {s}");
    ExitCode::from(exit)
}

/// The per-step CSV (module docs, "Output").
fn csv_text(
    rows: &[StepRow],
    armed_rows: &[ArmedRow],
    zones: Option<&ZoneTable>,
    deltas: &[(u64, u64)],
    tpn: f64,
) -> String {
    let n_cols = zones.map_or(0, ZoneTable::len);
    let mut s = String::with_capacity(rows.len() * (64 + n_cols * 16));
    s.push_str("step,wall_ns,manifolds,pairs,top_y,awake");
    if let Some(z) = zones {
        s.push_str(",void,colors,wide_colors,waves,g_ns,u_ns,r_ns,sys_sum_ns,disp_lane,worker_lane_max");
        for (name, _) in &z.systems {
            let _ = write!(s, ",sys_{name}_ns,sys_{name}_n");
        }
        for h in SPAN_ZONES.iter() {
            let _ = write!(s, ",{0}_ns,{0}_n", h.desc.name);
        }
        for h in COUNTER_ZONES.iter() {
            let _ = write!(s, ",{0},{0}_n", h.desc.name);
        }
    }
    s.push('\n');
    let n_ticks = zones.map_or(0, |z| z.systems.len() + z.spans.len());
    for (i, r) in rows.iter().enumerate() {
        let _ = write!(s, "{i},{},{},{},{},", r.wall_ns, r.manifolds, r.pairs, r.top_y);
        if let Some(a) = r.awake {
            let _ = write!(s, "{a}");
        }
        if let Some(a) = armed_rows.get(i) {
            let _ = write!(
                s,
                ",{},{},{},{},{},{},{},{},{},{}",
                u8::from(a.void),
                a.shape.colors,
                a.shape.wide_colors,
                a.waves,
                a.g_ns,
                a.u_ns.map_or_else(String::new, |u| u.to_string()),
                a.r_ns,
                a.sys_sum_ns,
                a.disp_lane,
                a.worker_lane_max
            );
            for (k, &(n, v)) in deltas[i * n_cols..(i + 1) * n_cols].iter().enumerate() {
                if k < n_ticks {
                    let _ = write!(s, ",{},{n}", (v as f64 / tpn).round() as u64);
                } else {
                    let _ = write!(s, ",{v},{n}");
                }
            }
        }
        s.push('\n');
    }
    s
}
