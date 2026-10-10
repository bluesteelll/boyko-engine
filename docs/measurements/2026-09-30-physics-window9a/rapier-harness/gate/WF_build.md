READY

# Implementation: Rapier 0.36 parity harness for the J-T pyramid

The harness is built and every untimed gate (g1–g4) passes, with no void run. Rapier's pile holds in all 18 trajectories; our trunk's pile fails. No timing was read, because the machine was not quiet. No harness process is left running, and D: has 96 GB free.

## Results

**Scene identity (g1).** The Rapier scene is ours, bit for bit:
- 1240 dynamic bodies plus 1 fixed floor.
- First box is (−15, 1, −15) and last box is (−1, 36, −1), both equal to our runner's.
- The pile's bounding box is (−16, 0, −16)..(14, 37, 14), as expected.
- Inverse mass 0.125 and inverse inertia 0.1875 differ from our constants by 0 ulp.
- All 1240×3 spawn coordinates are bit-equal to the lattice the F0 metrics script uses.
- Effective friction is 0.2 on every solver manifold, using max-combine like ours. Jolt's √ rule and Rapier's default average would also give exactly 0.2.
- The check can fail: moving box 0 by 1 ulp turns it red on exactly 1 body.
- Jolt's final pose puts body i on lattice site i for 1240/1240 bodies, so Jolt's body order is the same lattice order.

**Threads (g2).** Runs at W = 1, 2, 4, 8, 16 for 500 steps each:
- The pool-size read inside the install equals W at every step, and every body stays awake.
- Rapier's own pair, constraint and contact counters equal my recount at all 500 steps.
- **Final poses are bit-identical at every W, in both configs**, even on the build without enhanced-determinism. Repeat runs match, and so does per-step install.
- The determinism build also gives W1 = W8 bit-for-bit.
- Process CPU time over wall time is 0.99 at W1, 6.12 at W8 and 7.64 at W16. So the steps did run on several threads at once.

**Fidelity (g3).** Scored with the F0 metrics script: U plus the 8 F0 one-ulp seeds, at W1.

| | holds | boxes > 0.1 m | max drift |
|---|---|---|---|
| Rapier defaults | 9/9 | 76–89 | 210–285 mm |
| Rapier matched | 9/9 | 5–13 | 149–293 mm |
| Jolt 5.6 (one run) | holds | 0 | 51 mm |
| our trunk 16191fda | fails (26 boxes > 0.5 m) | 190 | 2183 mm |

**Equal work (g4).** Per-step means over steps [100, 500), W1:

| | manifolds | points | row-iterations |
|---|---|---|---|
| Rapier defaults | 8,070 | 32,229 | 354,674 |
| Rapier matched | 7,662 | 29,753 | 1,071,107 |
| ours | 4,468 | 16,554 | 595,935 |
| Jolt 5.6 | 8,489 | 31,112 | 565,790 + 62,224 position |

- Rapier's defaults do 0.60× our row-iterations and 0.63× Jolt's velocity rows.
- The matched config does 1.80× our row-iterations, close to the 1.68× predicted for our own 20 mm speculative-contact fix.
- So any future timing ratio against Rapier has to name the config and be read per row-iteration as well.

## Configurations
- **`--cfg rapier-default`**: `IntegrationParameters::default()` plus the scene invariants (dt 1/60, CCD off).
- **`--cfg matched`**: the research's closest match to our solver. 4 substeps × (1 biased + 2 relax passes), friction in the biased pass, Coulomb friction, 30 Hz / ζ10 softness on the floor too, 4 m/s bias cap, 0.02 speculative distance, 1 mm contact recycling, gyroscopic forces off.
- Rapier's angular-speed cap, its 400 m/s linear cap and its own contact generation cannot be matched; the README lists them.

## Files
- `D:/tmp/rapier-parity/Cargo.toml` and `Cargo.lock`: its own workspace root with its own lockfile. Pins `rapier3d = "=0.36.0"` with `parallel`, plus `simd8` (default) and `det` arms. Release profile uses fat LTO and codegen-units 1.
- `D:/tmp/rapier-parity/.cargo/config.toml`: x86-64-v3 and target dir `D:/wt/_targets/rapier-parity`.
- `D:/tmp/rapier-parity/rustfmt.toml`: sets `max_width = 110`. I added it because the harness had no format config.
- `D:/tmp/rapier-parity/build.sh`: builds both arms into `gate/bin/`.
- `D:/tmp/rapier-parity/src/main.rs`: the runner.
- `D:/tmp/rapier-parity/README.md`: how to build and run, every flag, and the scene transcription table (Jolt / ours / harness, file:line).
- `D:/tmp/rapier-parity/gate/`: `run_gates.sh`, `analyze.py`, `f0_metrics.py`, `gates.txt`, `gates.json`, the `bin/` executables, and every run's log, CSV and pose.
  - `f0_metrics.py` is a verbatim copy of the F0 ensemble's `metrics.py` (sha256 `ca8673d8…`).
  - `gates.txt` and `gates.json` are the gate results; `analyze.py` reads no timing column.

## Deviations from the brief
- **`simd-stable` no longer exists** in rapier3d 0.36; it was removed in 0.35. The maximum-performance arm is `parallel` + `simd8` (8-lane AVX2).
  - `simd8` cannot be built together with `enhanced-determinism` (rapier3d raises a compile error), so the determinism arm is a separate build.
- **Rayon pool**: taken from Rapier's own re-export (`rapier3d::rayon`) rather than a separate dependency, so it is guaranteed to be the pool Rapier steps on.
- **Density**: 1 (Rapier's default), giving m = 8 like our runner. Jolt's density is 1000; uniform mass scaling does not change the dynamics against a static floor.
- **CCD off in both configs**: `max_ccd_substeps = 0` is treated as a scene invariant, so the `rapier-default` config is defaults plus that.
- **Flags I added beyond the brief**:
  - `--install loop|step`. The default `loop` keeps the timed region to `step` alone; `step` is Rapier's own per-step install shape. Both give identical poses.
  - `--spawn-pose-out` and `--expect-pose`.
  - `--perturb B:AXIS[:DIR]`, with the same spec and log line as the F0 ensemble's perturbation knob.
  - `--label`.

## Unsafe blocks
None.

## Checks
- `cargo check`: success.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- Release builds with the stable msvc toolchain (rustc 1.98.1), fat LTO, codegen-units 1, x86-64-v3; each run reports AVX2, FMA and BMI2 as enabled.

## Known limitations
- Timing columns exist but were not read.
- Speculative-contact counts are approximate: on recycled pairs Rapier's contact distance can be stale.
- Jolt's fidelity numbers come from a single run, not an ensemble.

## Ready for code review