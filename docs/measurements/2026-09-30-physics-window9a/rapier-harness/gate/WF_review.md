VERDICT: NO-GO; CRITICAL=0; IMPORTANT=3

# Code review: Rapier 0.36 parity harness (D:/tmp/rapier-parity)

## Build checks
- `cargo clippy --locked --offline --all-targets -- -D warnings`: clean (cached, source unchanged since 07:26). `cargo fmt --check`: clean.
- Resolved features, checked with `cargo tree -e features`: rapier3d = default + parallel + simd8; parry3d = parallel + simd8. No `profiler`, `enhanced-determinism`, `solver-bounds-checks` or `unsync-callbacks`. There is only one `wide`, version 1.7.1. No global or parent cargo config overrides the profile.
- My structural runs: a 3-step W2 run was checked against the 500-step `g2/rapier-default_W1.pose` fixture. It gave rc=4, "mismatch … first differing body Some(0)". So the exit-4 path can fire, and the harness side of P2 now has its red control. No process is left running. D: has 74 GB free.

## Remarks

### Important

#### W1. Rapier's timed step includes a statistics walk that only runs because its counters are on; our timed row does not do this
**Where**: `src/main.rs:505` (`PhysicsPipeline::new()`); rapier3d `pipeline/physics_pipeline/mod.rs:110` (`counters: Counters::new(true)`); `dynamics/solver/staged_island_solver/init.rs:601-610`; `config_json` at `main.rs:786-823`.
**Problem**:
- `PhysicsPipeline::new()` turns the counters on, and the harness never turns them off.
- With counters on, `init_and_solve` walks every solver manifold on every step (`graph.buckets()…map(|r| store.get(*r).data.solver_contacts.len())`). Each lookup follows the pair, then the manifold (`manifold_store.rs:100-122`), with no prefetch.
- The walk runs serially on the calling thread before the parallel phase at `init.rs:662`, and it runs inside our `Instant` pair.
- It is the only work the counters flag gates. Without `profiler` the timers are no-ops (`counters/timer.rs:33-56`).
- Our J-T runner does its counts after the pair closes (`jolt_parity_pyramid.rs:22-25`).
- The SUMMARY `config` does not record the setting, so the V5 config pin cannot see it.
**Failure**: in the 9a RP-M rows at W8/W16, Rapier's wall includes a serial pass over about 7,662 manifolds per step that the disarmed J-T row does not have. This adds to Rapier's serial fraction and pushes R-WALL-M / R-ROW-M towards "ours faster", which is the direction a false win would take.
**Confidence**: CONFIRMED that the walk runs every timed step inside the pair (code path traced). The size of the effect is PLAUSIBLE; measuring it needs timing.
**What to do**:
- Make the choice explicit and record it in the SUMMARY `config` (for example `counters_enabled`).
- The natural shape: turn counters off unless `--receipt` is given. The receipt twins keep the recount gate. Counters are write-only (nothing in the dynamics reads `counters.solver.*`), so the pose fixtures still identify the trajectory.
- Whether RP-D, the "product bar", keeps Rapier's default of counters on is the orchestrator's call. It must be pinned either way.

#### W2. V3 in the window rows requires a CSV column that the timed command lines never produce
**Where**: `main.rs:853-893`. The `pool_threads` column is written only inside `if receipt` / `if let Some(c) = &r.counts`.
**Problem**: V3 requires "CSV `pool_threads == W` on all 500 rows". V9 says timed rows run without `--receipt`. Without `--receipt` the CSV is only `step,wall_ns`; my 3-step run shows exactly this.
**Failure**: a strict window validator would void every Rapier timed process (all 144). A lenient one would skip the clause, and the per-step pool check would then pass without checking anything.
**Confidence**: CONFIRMED.
**What to do**: either always write `pool_threads` to the CSV, or remove the CSV clause from V3 and rely on the SUMMARY's `pool_threads_min/max`. The per-step read already happens in loop mode whether or not `--receipt` is set (`main.rs:703-707`).

#### W3. There is still no 4-lane build, and "maximum-performance arm" is an assumption
**Where**: `build.sh:15-18`, `Cargo.toml [features]`. This is the analyst's P1, still open.
**Problem**: the harness has only rs8 (plus `det`, which is not timed). There is also a structural reason simd8 can lose at high W. The solver's worker cap is `num_workers.clamp(1, (num_two_body / SIMD_WIDTH / 16).max(1))` (`init.rs:258-273`). Under simd8 it only allows all 16 workers from about 2,048 manifolds, against about 1,024 under simd4.
**Failure**: during the landing steps of [0,100) (225 manifolds at step 0), rs8 runs fewer solver workers at W8/W16 than rs4 would. A "ours faster" result against rs8 alone could then be a result against a handicapped Rapier. With only rs8 built, R-WALL-D's "every build measured" rule is met trivially.
**Confidence**: CONFIRMED for the clamp formula. PLAUSIBLE that it changes a claim.
**What to do**: add the `--locked --no-default-features` arm, pin its sha, and record the RD4 and RM4 fixtures, as P1 states.

### Optional
- **O1. `--locked` is missing** (`build.sh:15,17`, analyst's P4). The risk is low: the lock only re-resolves if the manifest changes. Add it anyway for parity with our builds.
- **O2. The binaries' source is not pinned.** `D:/tmp/rapier-parity` is not a git repo, so nothing ties `24808408…` to a particular `main.rs`. The W1/W2 rework forces an rs8 rebuild, so the rs8 sha pin in the 9a rows has to be taken again (V4 would otherwise void every rs8 run). Suggestion: record `sha256(main.rs, Cargo.lock)` in the SUMMARY, or put the harness under git.

## Checklist items with no reachable failure
- **Scene**: positions use our expression operand for operand; the bits match the F0 lattice (g1 plus the 1-ulp mutant red). The floor is 50×1×50 at y = −1. Mass 8 and inverse inertia 0.1875 match at 0 ulp. Friction is 0.2 under Max combine, restitution is 0, damping defaults to 0, and every box has `can_sleep(false)`. Gravity and dt match. Substeps are 4 in both configs.
- **CCD off in rapier-default**: in 0.36 CCD only activates when a body moves more than half its thinnest extent (1 m) in one step (`ccd_solver.rs:64-95`). These boxes move at most about 5 cm per step, so default CCD would do essentially nothing here.
- **Timed region**: in loop mode the pair holds only `pipeline.step`. The Row push goes into a preallocated Vec, and nothing is printed or allocated by the harness inside the pair.
  - The deferred BVH task is spawned in `detect_collisions` (`solve.rs:91-98`) and joined in `update_moved_collider_aabbs` (`substep.rs:246`, called at `:707`) in the same step. No Rapier work crosses a step boundary.
- **Pool**: with `thread_pool` set to `None`, Rapier steps on the caller's pool (`mod.rs:217-236`). `rayon::current_num_threads()` sizes the solver (`solve.rs:382`). `broadcast`, `in_place_scope` and `par_*` all run on the current registry. Under `pool.install` the loop is on worker 0 and the main thread is blocked, not spinning.
- **Build**: `profiler` is off, so no timer overhead. `-C target-cpu=x86-64-v3` applies to all dependencies, and `wide` f32x8 compiles to AVX.
- **Counts**: `solver_manifolds()` is what the solver sees (`contact_pair.rs:312-318,499`), and it matches Rapier's own counters at 500/500 steps. The row formula matches `worker.rs:269/616/713` and `solve.rs:98-100`.
- **Pose format**: glamx `Rot3 = glam::Quat`, so the order is [x,y,z,w]. Positions are the body origin, which is also the centre of mass. The layout is byte-identical to our `pose_bytes`.
- **P3 is not needed**: `loop_on_pool_worker` cannot be false inside `pool.install` (`main.rs:975-979`), so a void on it could never fire.

## Positive
- The scene-identity gate can fail, and has been shown to.
- Poses are bit-identical across W 1–16 without enhanced-determinism.
- The counter-versus-recount cross-check.
- Rapier gets its pool through its own re-export, and the harness keeps its own lockfile and workspace root.

## Open questions and notes for audit.md
- **The analyst's V9 reason is wrong.** It says the deferred BVH task is "joined in the next step"; it is joined in the same step (see above). The rule itself (timed rows without `--receipt`) still holds for a different reason: the `count()` walk changes the cache state between steps. Please correct the sentence before the AUDIT text is saved verbatim.
- **Not in the README's list of unremoved differences:** `matched` keeps `normalized_allowed_linear_error = 0.005`. A grep of `boyko_physics/src` found no equivalent slop on our side. Is this a real difference? The row counts are unaffected either way.
- **Build asymmetry:** Rapier is built with codegen-units 1 per the brief; ours is fat LTO with codegen-units 16. Our own `Cargo.toml` notes that fat + cgu 1 measured slower for our engine. Keep this as a stated condition on every claim.

Files: D:/tmp/rapier-parity/src/main.rs, D:/tmp/rapier-parity/build.sh, D:/tmp/rapier-parity/Cargo.toml, C:/Users/flint/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rapier3d-0.36.0/src/dynamics/solver/staged_island_solver/init.rs, C:/Users/flint/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rapier3d-0.36.0/src/pipeline/physics_pipeline/{mod.rs,solve.rs,substep.rs}