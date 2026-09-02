# FMA and determinism — oracle inventory, practices survey, the rejected scheme, the re-bless list

Companion to [FMA-DETERMINISM.md](FMA-DETERMINISM.md) (the decision) and
[FMA-DETERMINISM-MEASUREMENTS.md](FMA-DETERMINISM-MEASUREMENTS.md) (the numbers). This file is the
record: what every determinism gate in `boyko_physics` asserts and whether fusion could break it;
what the record says those gates have caught; what other engines do and which parts transfer to a
Rust engine; the feature scheme priced in full so its rejection is of a concrete thing; and the
re-bless inventory kept ready for the day a deliberate solver-math change is made.

---

## Oracle inventory

"Would fusion break it" has three honest answers: **no** (a self-comparison that fusion cannot
see), **only-if-paths-disagree** (a SIMD-vs-scalar gate that survives matched fusion and fails on
one mismatched site), and **yes** (a pinned literal that moves whenever the value moves, or a
contract with a party outside the engine).

**Where each gate runs is a separate question, and revisions 1 and 2 each answered it wrongly in
opposite directions** — revision 1 assumed both CI profiles carried the baseline, revision 2
measured a `RUSTFLAGS` value CI no longer sets and concluded CI had never built the AVX2 arm.
Revision 3 reads the tree: since commit `cced895a` (2026-09-02) every `ci.yml` job that builds
`boyko_physics` restates `-C target-cpu=x86-64-v3` inside its `RUSTFLAGS`, the Miri job keeps a
plain `-D warnings` and never builds the crate, and the root package's
`tests/isa_baseline_census.rs` (five features, size pin, negative control) runs on every
`--workspace --all-targets` leg. Local and CI are therefore the **same configuration** — the AVX2
one — and the only invocation that runs no ISA guard at all is `cargo test -p boyko-physics`,
which is what the decision file's crate-local Test 2 exists for. Per gate:

| Gate | Local (`cargo test -p boyko-physics`, config in force) | CI (after `cced895a`) |
|---|---|---|
| Worker-count / run-to-run self-comparisons; both pinned goldens; `colored_acceptance_o5`; `sdf_collision`; `box_box` anchors; `integrate_determinism`; `ke16_app1_solve_in_system_bit_identity` | runs, AVX2 configuration | runs, AVX2 configuration |
| `solver::colored::tests::{simd_solve_bits_match_scalar, cone_adversarial_differential_test_1c, degenerate_lane_differential_test_1d, cohort_shape_proptest_bit_exact_and_non_vacuous}` | runs | runs |
| `sdf_simd::o9_kernel_tests::*` including `sdf_simd_has_no_fma_or_approx_callsites` | runs | runs |
| `tests/colored_simd_parallel_o7`, `tests/colored_acceptance_simd_o7` | run | run |
| `solver::simd::tests::*_simd_bits_match_scalar`, `degenerate_quat_lane_matches_scalar`, `adversarial_inputs_simd_bits_match_scalar`; `tests/simd_o1` | run, SIMD vs scalar | run, SIMD vs scalar |
| `solver::simd::tests::solver_simd_has_no_fma_or_approx_callsites` | runs | runs |
| Root `tests/isa_baseline_census.rs` | **does not run** under `-p boyko-physics` (root package) | runs on every `--workspace --all-targets` leg |
| **New** core-wide census (decision file, section "The gate") | runs | runs, unconditionally — no `cfg` |
| **New** crate-local AVX2-arm guard | green; red on any `-p` run whose environment dropped the arm | green |

What a build that *does* lose the flag loses is measured in the measurements file, section "Flag
delivery" (136 → 127 unit tests, nine named; two integration files; six gates degraded to
scalar-equals-scalar). That is the hazard the root census and Test 2 exist for, not the state of
any leg today.

| Gate | Asserts | Would fusion break it | Why |
|---|---|---|---|
| `colored_tests::parallel_solve_is_bit_identical_across_worker_counts` | Full solver-state `u32` snapshot on a forced-collision dense scene equal across parallel {1,2,4,8} workers and equal to the single-threaded colored solve; anti-vacuity assert that the scene moved | **no** | Self-comparison of one code path against itself at different worker counts. This is the crate's race detector and survives fusion untouched. |
| `colored_tests::colored_columns_snapshot_is_byte_identical_across_workers_and_runs` | The 31-column `ContactColumns` snapshot byte-identical run-to-run, across {1,2,4,8} workers, and on repeat; anti-vacuity gate that the widest colour clears `MIN_PARALLEL_SLOTS_PER_COLOR` | **no** | Same — no pinned value. |
| `colored_tests::simd_solve_bits_match_scalar`, `simd_solve_width_only_matches_scalar_step`, `cone_adversarial_differential_test_1c`, `degenerate_lane_differential_test_1d`, `cohort_shape_proptest_bit_exact_and_non_vacuous` | `solve_color_avx2` output (velocities + impulse columns, `to_bits`) equals `solve_color` exactly over adversarial cone-clamp / degenerate / static-A / sentinel-B / `k <= 0` corpora and a 200-shape proptest | **only-if-paths-disagree** | Measured both ways: matched sites 0/29 752; one mismatched site 2 799/29 752 at up to 256 ULP. |
| `solver::simd::tests::{refresh_inertia, apply_gravity, position_integrate}_simd_bits_match_scalar`, `degenerate_quat_lane_matches_scalar`, `adversarial_inputs_simd_bits_match_scalar` | Per-body `Mat3` / `Vec3` / `Quat` `to_bits` equality between the O1 AVX2 kernels and their scalar oracles over random, tail and adversarial (denormal / `-0.0` / near-`MIN_POSITIVE`) corpora | **only-if-paths-disagree** | Measured for `refresh_inertia`: matched sites 0/90 072 over all nine tensor elements. |
| `tests/simd_o1` | The full pipeline with `PhysicsConfig.simd = true` is bit-identical to `simd = false` for every body's final `RigidBody` | **only-if-paths-disagree** | The integration form of the row above. |
| `sdf_simd::o9_kernel_tests::x8_bits_eq_scalar_bits_widened_proptest`, `assert_lane_bit_exact`, `x8_inert_lanes_do_not_leak` | Every lane of `sdf_edit_list_x8` is `to_bits`-identical to `boyko_sdf_math::sdf_edit_list` at that lane's point; inert lanes cannot influence live lanes | **yes** | The scalar oracle is the frozen CPU↔GPU leaf. Fusing the x8 side requires fusing the leaf, which its header forbids for the ±2/255 GPU goldens in `boyko_rhi_vulkan`. |
| `tests/bodytype_determinism_golden::bodytype_determinism_golden_hash_is_stable` | A pinned splitmix hash `GOLDEN` of every body's pose and velocities after 90 steps of a fixed mixed scene on a single-threaded pool | **yes** | A pinned literal moves when the value moves. The file's own contract text says the hash "MUST NOT change" and, in the assertion message, offers the escape "If this is a deliberate solver-math change, re-capture the golden." |
| `tests/soft_colored_sp4_baseline::serial_guard_is_byte_identical_to_pre_guard_golden` | The soft colored serial solve reproduces `SP4_SERIAL_GOLDEN`, hard-coded `f32` bits captured before the pinned-write guard | **yes** (soft kernels only) | Pinned literal; the scene is `spawn_soft` only, so a rigid-only change leaves it alone. Generator `print_serial_golden` exists in the same file. |
| `tests/ke16_app1_solve_in_system_bit_identity::solve_in_scheduled_system_is_bit_identical_to_the_serial_oracle` and the `_at_1_2_4_and_8_workers` sibling | The solve inside a scheduled system on a registered worker equals the serial oracle at 1/2/4/8 workers; asserts values, never timing | **no** | Self-comparison across dispatch routes. |
| `tests/colored_rigid_scratch_determinism::{scalar_colored_one_vs_n_worker_bit_identical, scalar_colored_serial_run_to_run_byte_identical}` | Serial == {1,2,4,8}-worker snapshots; two serial runs byte-identical | **no** | Self-comparison. |
| `tests/integrate_determinism::integrate_is_deterministic` (proptest) | Two runs of the integration over a multi-threaded pool give `to_bits`-equal pose and velocities per body | **no** | Self-comparison. |
| `colored_tests::colored_columns_snapshot_matches_pre_p2_vec_baseline` | The 31-column snapshot equals a baseline file at `D:/tmp/p2_baseline_columns.txt` | **only-if-paths-disagree**, but vacuous | Silently `return`s when the file is absent, so on any machine but the one that captured it the test cannot fail. Pre-existing, unrelated to FMA; surfaced to the owner in the decision file. |
| `solver::simd::tests::solver_simd_has_no_fma_or_approx_callsites`, `sdf_simd::o9_kernel_tests::sdf_simd_has_no_fma_or_approx_callsites` | Source census over the file's own non-comment lines: no `_mm256_`/`_mm_` × `{fmadd, fmsub, fnmadd, fnmsub, fmaddsub, fmsubadd, rsqrt, rcp}` × `_ps(` call site and no `mul_add(`; needles assembled at runtime; non-vacuity witness `_mm256_mul_ps(` | **yes** — by design | These are what replaced the `compile_error!` guards on 2026-09-02 (commit `cced895a`). They gate the right proposition (a source property) but over two files only, and their `mul_add(` needle does not match `mul_add_relaxed(` or any `algebraic_*(` spelling (the real prefix-form names, probed on both toolchains — measurements file, section "API spelling probes"). The core-wide census in the decision file is their strict superset; they stay unchanged. |
| Tolerance acceptance gates — `tests/colored_acceptance_o5` (COM drift, restitution bounds, resting creep, penetration, box-stack drift/sink/jitter, static-friction slide), `tests/colored_acceptance_simd_o7` (distance and COM drift on the O7 path), `tests/sdf_collision` (distance, unit gradient, union, carve, separation), `narrowphase/box_box` manifold `to_bits` and 1e-4 anchors | Physical plausibility within fixed tolerances | **probably not, but must be re-argued** | The converged value moves by up to 6 848 ULP on individual impulses; a tolerance that "still passes" has not been shown to pass for the right reason until it is re-run and inspected. |

---

## What the bit oracles caught

The honest pricing of "sharpness", from the O4–O11 and Stage-P record:

- **The SP4 pinned-particle race was not caught by the {1,N} bit oracle and could not have
  been.** `docs/archive/PHYSICS-O11-SP4-PLAN.md` states it: without the guard, two workers both
  run `pos[pinned] += +0.0` on the same row — "a value-benign but real data race, INVISIBLE TO
  SNAPSHOT TESTS, visible only to Miri-TB/loom". The plan's gate list names Miri-TB as "the ONLY
  oracle for the pinned-write race".
- **The O7 cross-worker gather read (the review's CRITICAL finding) was the same class.** The
  per-rank gather clamped an exhausted lane's slot to the global column length and read a slot a
  sibling worker was writing — "a value-benign (blendv-discarded) but real cross-thread data race
  (UB)". Found by a four-lens adversarial review, fixed with an in-span `debug_assert!`, not by any
  oracle.
- **The code admits the limit.** `solver/colored.rs`, at the `is_dynamic_row` linchpin, says the
  colouring predicate and the solve-guard predicate must agree over the same `inv_mass` snapshot,
  "else the guard could permit writing a row the coloring believed shared (a cross-worker race the
  {1,N} bit test cannot detect)". The mitigation is structural, not oracular.
- **What the bit gates did catch:** nothing wrong in the O7 kernel across 7 000+ adversarial
  trials and 12 756 cone clamps — which is what turned "nothing was wrong" from a hope into a
  finding — and one real defect on the P2 path, a `cohort_shape` registration-order bug, surfaced
  while bringing the 31-column byte gate up.
- **A defect in an oracle itself, the most instructive entry.** Commit a0336bd9 "soft-colored {1,N}
  oracle non-vacuous in release": the parallel-dispatch counter was incremented only under
  `cfg(debug_assertions)`, so under `--release` the anti-vacuity guard tripped before the bit
  assertion ran — "the SP4-soft race fix was NEVER VERIFIED in the optimized multi-worker profile
  where the data race actually bites". Nominal sharpness, zero reach.
- **Summary:** neither of the two real cross-worker races in the record had a value signature,
  so no value oracle — bit-exact or tolerance — could have caught them; Miri-TB and adversarial
  review did. The one defect the byte gates did surface (the `cohort_shape` registration-order
  bug) is of a class a tolerance test *can* miss, because a wrong-but-plausible ordering converges
  to a plausible answer; that single entry is enough to keep the byte gates rather than replace
  them with tolerances. Beyond it, what the bit oracles bought is the 0 %-gate: P1, P2, O1, O6 and
  O7 each landed as provably value-neutral because "this refactor changed nothing" is a
  proposition only a bit test can settle. That is a refactoring instrument, and it is worth keeping
  because it is free — nothing in the decision spends it. (Revision 1 summarised this section as
  "no instance … that a tolerance test would have missed", which its own `cohort_shape` entry
  contradicts; that sentence is withdrawn so it is not quoted to justify a future replacement of
  a bit gate by a tolerance.)

---

## Practices survey — what other projects do, and what transfers

| Project | What it promises | How it handles FMA | Mechanism | Transfers? |
|---|---|---|---|---|
| **Box2D 3.1+** (Catto) | Algorithmic, multithreaded and cross-platform determinism, on by default, no settings; rollback determinism explicitly not promised | Bans it outright: `-ffp-contract=off` for GNU/Clang and `/clang:-ffp-contract=off` for clang-cl, in the library's own CMake. Names three troublemakers: fast-math, FMA, trigonometry. No explicit FMA speed path anywhere. | Compiler flag; no fixed point (rejected for overflow and speed); worker-count independence via thread-local bit arrays merged by bitwise OR | **The reason does not transfer; two findings do.** The ban targets compiler contraction, which Rust does not perform. The bit-array-OR merge is the property our worker-count oracles already assert. Transcendentals (`atan2f` diverged; `sinf`/`cosf` did not) are the real cross-platform hazard, not arithmetic. |
| **Jolt** (Rouwe) | Default tier: deterministic given the same binary and the same API call order, independent of thread count. `CROSS_PLATFORM_DETERMINISTIC` tier: bit-exact across compiler/OS/architecture/word size | Ships explicit FMA in the fast tier (`JPH_USE_FMADD`, auto-defined from the ISA but only under `#ifndef JPH_CROSS_PLATFORM_DETERMINISTIC`, comment "FMA is not compatible with cross platform determinism") | One wrapper `Vec4::sFusedMultiplyAdd` with two lowerings behind a CMake option; the deterministic tier also needs `-ffp-model=precise`/`/fp:precise`, `-ffp-contract=off`, Jolt's own `Sin`/`Cos`/`QuickSort`/`BinaryHeap`/`Hash`; measured cost ~8 % | **The exact shape of the owner's second sentence, and the exact shape rejected.** Jolt keeps no fused-equals-unfused bit identity — its fast tier promises same-binary determinism only. Hazard to note: its NEON branch uses `vmlaq_f32`, which ACLE defines as the *unfused* multiply-accumulate (`vfmaq_f32` is the fused one) — unverified here, but the class "a wrapper named fused whose lowering is not" is real. |
| **Rapier** (dimforge, Rust) | Cross-platform bit-level determinism behind `enhanced-determinism`, conditional on IEEE-754-2008 conformance and routing float construction through nalgebra's `RealField` | Does not mention FMA anywhere | `enhanced-determinism = ["simba/libm_force", "parry3d/enhanced-determinism"]` — a dependency swap forcing software transcendentals; documented incompatibility with `simd8` (lane width changes grouping). The often-repeated `parallel` incompatibility does not appear in the current manifest or guide. | **High.** A production Rust physics engine whose selling point is determinism spends its whole budget on the transcendental library and lane width, and none on FMA. Its mechanism (a feature that swaps a dependency, no `cfg` inside kernels) is the Rust-idiomatic shape if a feature were ever wanted. |
| **PhysX 4/5** | "Limited deterministic simulation": same platform, same insertion order, same release; independent of worker-thread count; no cross-platform claim | Not addressed | `PxSceneFlag::eENABLE_ENHANCED_DETERMINISM` buys island independence ("the simulation of an island will be identical regardless of any other islands"), at a stated performance cost | **Moderate.** "Enhanced determinism" means two unrelated things in two engines (PhysX: island/insertion-order invariance; Rapier: cross-platform bit-exactness); neither is FMA. Island independence is the property our coloured solve could most plausibly violate and is worth its own gate some day. |
| **Bullet** | Nothing official; forum consensus: not deterministic by default, achievable per platform with solver reset + fixed insertion order | No project position; user-side `-ffp-contract=off`, `-mfpmath=sse`, `/fp:strict` | Ordering | **Low.** Recorded as the counter-example: without a contract, determinism advice accumulates as folklore. Weakest row — primary sources (a Ubisoft PDF) could not be read. |
| **Unity DOTS / Burst** | Deterministic on one CPU architecture; cross-architecture is a goal Burst does not yet deliver | No per-operation policy | The community's only general answer is soft float | **Nil as technique, useful as calibration:** the industry's fully general answer to cross-vendor reproducibility is to stop using the FPU. Cross-vendor bit-exactness is not a property to chase casually, and this engine never promised it. |
| **Factorio** | Lockstep multiplayer across heterogeneous clients (500-player server, mixed OS and CPU) | No FMA-specific statement; risk model: "compiler optimizations, what specialized hardware the compiler decides to use, and when values are moved between registers and memory" | Floats, not fixed point; custom trig; integration tests; their real bug was an ambiguous sort comparator | **Moderate.** Their bug class (comparator ambiguity — same as Jolt's `std::push_heap` tie-break) is our constraint-graph colouring and contact-ordering surface, and it matters more than FMA. Fixed point is retired as an answer at production scale. |
| **RTS lockstep literature** (AoE, Supreme Commander) | Full-state reproduction from inputs; deterministic replay | Not discussed | Fixed turns; strict IEEE mode ("comes with a performance cost"); the worked desync is a dangling pointer, not rounding | **High as prior, low as technique.** All of it solves cross-machine lockstep, which this engine does not have. The ranking transfers: uninitialised memory, iteration order and comparator ambiguity outrank rounding. The "AoE used fixed point" claim is unverified and not asserted. |
| **Bruce Dawson, "Floating-Point Determinism"** | Analysis, not a promise | FMA "effectively [has] infinite intermediate precision … gives different results than machines that don't have an fmadd instruction" — an availability axis, not a vendor axis | Case separation: same binary, same processor → "altered FPU settings, uninitialized variables, and non floating-point specific sources"; cross-platform → x87 state, optimisation level, capability-dispatching libraries, transcendentals | **Direct.** His same-binary list is our situation and FMA is not on it. His three residual risks map to: MXCSR (the grep was done in revision 3 — zero sites in `crates/` read or set it, so the engine assumes the process default and the decision file specifies a debug-build probe to check it), uninitialised memory (Miri), and non-FP nondeterminism (reduction order — what our oracles gate). |
| **Clang / GCC** | Language-level control of contraction | Clang: `-ffp-contract=on` is the C/C++ default (fuse within a statement); GCC: `-ffp-contract=fast` for C++ and non-strict C (fuse across statements). A documented case shows `gcc -O3 -march=haswell` forming an FMA only after inlining — same source, same flags, last two bits differ. | `-ffp-contract`, `#pragma STDC FP_CONTRACT` | **Validates the pass's central distinction and shows it is language-specific.** In C/C++ contraction and explicit fusion cannot be separated by source discipline; every C/C++ FMA ban above targets a mechanism Rust does not have and is not evidence against explicit `mul_add` in Rust. |
| **Rust / rustc** | `f32::mul_add` is IEEE `fusedMultiplyAdd`, "guaranteed not to change" — a stability guarantee | Never contracts; fusion only where a human writes `mul_add` (→ `llvm.fma`) or a fused intrinsic. Re-confirmed on this checkout by disassembly. | Per-site source calls. Adjacent and **not** oracle-safe, spellings probed on this box: the `float_algebraic` family is `algebraic_{add, sub, mul, div, rem}` (prefix form; unstable on 1.97.1, ungated on nightly 1.100.0 — stabilised), and `mul_add_relaxed` (`llvm.fmuladd`, "1 or 2 roundings, nondeterministically") exists on neither toolchain yet. | **Decisive.** Explicit `mul_add` cannot break a worker-count oracle. The two adjacent features must be gated against now: the current censuses' `mul_add(` needle matches neither `mul_add_relaxed(` (verified: the substring is absent) nor any `algebraic_*(` — and revision 2's needle for the latter was spelled for methods that do not exist, which the decision file's per-needle synthetic witness now prevents. |
| **IEEE 754-2008 §5.4.1** | `fusedMultiplyAdd(x, y, z)` = `(x·y)+z` "as if with unbounded range and precision, rounding only once" — uniquely determined per rounding mode | The standard that makes explicit FMA portable: Intel `vfmadd*`, AMD's implementation, AArch64 `FMLA` agree bit-for-bit | What breaks it in practice: non-default rounding mode (MXCSR/FPCR), FTZ/DAZ subnormal flushing, an intrinsic that is not actually fused on some target, an unaudited software `fma()` | **Settles the cross-vendor sub-question:** yes, explicit FMA is the same answer on Intel and AMD, subject to default rounding and no flushing — nearly free for one x86-64-v3 binary. It is a *different* answer from the split form by design; that difference is the entire content of the SIMD-equals-scalar oracle. |
| **uops.info** (YMM, f32) | Measurements | `VFMADD213PS` lat 4–5 / tp 0.50 on Haswell through Zen 4 and Alder Lake-P (Alder Lake-E 6 / 1.00); `VMULPS` 3–5 / 0.50; `VADDPS` 2–4 / 0.50–1.00 | Read by dependency structure: throughput-bound → FMA is ~2× on the FP ports; latency-bound serial accumulate → split is strictly faster on Haswell (3 vs 5), Zen 2/3/4 (3 vs 4–5), Alder Lake-P (2 vs 4), a wash on Skylake/Ice Lake | **Direct and cautionary.** The sign of "FMA is faster" is per-loop. The measured colored-solve result (no prize) is the latency-bound case; the measured inertia result (1.16×) is the throughput-bound case. |
| **Bevy** discussion #2480 | Nothing shipped; an audit of what would be needed | FMA and fast-math not mentioned | Proposed `enhanced-determinism` for `bevy_math` on glam's `libm` flag (the lever Rapier shipped); the five enumerated sources are platform FP, system order, entity iteration order, PRNG, command/event order | **Moderate.** Four of five items are ordering, owned by our scheduler and archetype row order — where a determinism regression in this engine is most likely to originate. |

### Sources

Primary sources read in full or fetched: box2d.org/posts/2024/08/determinism/;
erincatto/box2d `CMakeLists.txt`; jrouwe/JoltPhysics `Jolt/Core/Core.h`, `Jolt/Math/Vec4.inl`,
`Docs/Architecture.md`, discussion #617; rapier.rs/docs/user_guides/rust/determinism/ and
dimforge/rapier `crates/rapier3d/Cargo.toml`; PhysX 5.3 and 3.4 Rigid Body Dynamics manuals;
randomascii.wordpress.com/2013/07/16/floating-point-determinism/; forrestthewoods.com "Synchronous
RTS Engines and a Tale of Desyncs"; clang.llvm.org UsersManual `-ffp-contract`; gcc.gnu.org
Optimize-Options `-ffp-contract`; siboehm.com/articles/23/Inlining-FMA-FP-consistency;
doc.rust-lang.org `f32::mul_add`; rust-lang/rust #151770, #136468; uops.info pages for
`VFMADD213PS`, `VMULPS`, `VADDPS` (YMM); bevyengine/bevy discussion #2480. Search-snippet level only
(marked as such where cited): Intel's "Consistency of Floating-Point Results" (HTTP 403), the Arm
ACLE `vmlaq`/`vfmaq` entry, Factorio's floating-point FFF posts, Unity soft-float community work,
the Bettner & Terrano GDC 2001 paper and the Ubisoft Bullet deck (both PDFs unreadable in the
session that surveyed them).

---

## The feature scheme, priced in full

Rejected in the decision file; specified here so the rejection is of a concrete design and so a
future campaign that *does* have a prize can start from it rather than from a blank page.

**Feature names.** `boyko_physics` gains `fp-split` and `fp-fused`. `fp-split` is in
`default = [...]`. Mutual exclusion:

```rust
#[cfg(all(feature = "fp-split", feature = "fp-fused"))]
compile_error!("boyko_physics: `fp-split` and `fp-fused` are mutually exclusive; \
                build with `--no-default-features --features fp-fused` for the fused arm");
```

A build with neither feature (`--no-default-features`) behaves as `fp-split` — the split arm is the
unconditional fallback, never the fused one, so a misconfigured downstream gets the tested engine.

**One wrapper, both arms.** `boyko_math` gains `#[inline] pub fn mla(a, b, c) -> f32` and `mls`,
with bodies `a * b + c` / `a * b - c` under `fp-split` and `a.mul_add(b, c)` /
`a.mul_add(b, -c)` under `fp-fused` (the feature forwarded from `boyko_physics`), plus the
`__m256` twins `mla_x8` / `mls_x8` in `solver/simd.rs` with `_mm256_add_ps(_mm256_mul_ps(..))` /
`_mm256_fmadd_ps`. **Every** fusible site in the scalar oracles (`Vec3::dot`, `Vec3::cross`,
`Mat3::mul_vec`, `impl Mul for Mat3`, `contact.rs::{effective_mass, point_velocity,
apply_impulse}`, the `solve_color` deltas, the soft kernels) and every site in the x8 helpers is
routed through the wrapper. That is what makes "both arms fuse at the same places" true by
construction rather than by care — and it is a rewrite of every arithmetic line in the core.

**Censuses become configuration-aware.** Under `fp-split` they ban exactly what they ban today
(plus the widened needles). Under `fp-fused` they ban the fused intrinsic family and `mul_add(`
everywhere in the core **except** the four wrapper bodies, which are matched by name and skipped;
and a second check asserts that no non-comment line in the core contains a bare `* ` … `+ `
three-term pattern outside the wrappers — which is not decidable by a line grep (expressions span
lines; `+` has other uses) and would therefore be a review rule, i.e. the thing this campaign's
memory calls "a rule with no gate".

**Goldens in both arms.** `bodytype_determinism_golden::GOLDEN` and
`soft_colored_sp4_baseline::SP4_SERIAL_GOLDEN` each become a `cfg`-selected pair; the tolerance
gates in `colored_acceptance_o5`, `colored_acceptance_simd_o7`, `sdf_collision` and `box_box` are
re-run and re-argued in the fused arm.

**CI.** `.github/workflows/ci.yml` gains, on both the debug and the release matrix legs:

```text
cargo test -p boyko_physics --no-default-features --features fp-fused --all-targets
cargo test -p boyko_physics --no-default-features --features fp-fused --all-targets --release
```

and the clippy leg gains the same feature set once. Without these legs the fused arm is a path
that compiles and is never run — this repository's record on `hwrt` ("default = false, and every
gate in this tree…" in `docs/OPEN-QUESTIONS.md`), on the a0336bd9 release-vacuity fix and on the
2026-08-10 `--no-fail-fast` finding is a record of exactly that failure. As `ci.yml` stands
(after `cced895a`) both legs would inherit the workflow-wide `RUSTFLAGS` with the ISA flag
restated and would build the fused **AVX2** arm; a revival that gives them a job-level
`RUSTFLAGS` must restate `-C target-cpu=x86-64-v3` in it, and the root
`tests/isa_baseline_census.rs` turns forgetting that into a named red rather than a smaller
engine.

**What it buys, measured:** 0.26–0.35 % of a physics step (the inertia refresh), nothing on the
solve, nothing on the SDF; and a 5–9 % regression on the scalar solve *if* the fused arm were ever
made the default. **Rejected on the number.**

---

## Re-bless list — kept on record, empty for this decision

Nothing fuses, so nothing moves. The inventory below is what a future *deliberate* solver-math
change (the only legitimate reason to touch a pinned literal) must walk, with the reviewer's test
for telling a legitimate re-bless from a masked regression.

| Pinned datum | Where | Moves if | Re-blessed by | Generator |
|---|---|---|---|---|
| `GOLDEN` (`u64` splitmix hash) | `tests/bodytype_determinism_golden.rs` | any rigid-solver value change | the developer of the value-changing commit, in a **separate** commit | run the test with `--nocapture`; the failure message prints `got {actual:#018X}` |
| `SP4_SERIAL_GOLDEN` (`[(u32; 6); 5]`) | `tests/soft_colored_sp4_baseline.rs` | any soft-kernel value change | same | `print_serial_golden` (`--nocapture`) |
| `p2_baseline_columns.txt` | `D:/tmp` (outside the repo) | any colored-solve value change — but only on the machine that has the file | owner call (decision file, "The owner's remaining calls") | `run_dense_columns_in_pool(400, 12, true, 4)` |
| Criterion baselines for `parallel_solve`, `ke16_solve_in_system`, `simd_o1`, `colored_solve` | local `target/criterion` | any value or codegen change | nobody — they are comparison points, not gates; re-take | the benches |
| The plan's acceptance figures "≥ 1.8× (SIMD) atop the parallel scaling on a 10k pyramid" and "criterion ≥ 0.6× linear to 4 workers" | `docs/OPTIMIZATION-PLAN-PHYSICS.md`, sections "Phase O6" / "Phase O7" / "Production-ready" | a change in what the kernels compute or how fast | the orchestrator, by re-measurement, with the new number written next to the old | — |
| Tolerance gates | `tests/colored_acceptance_o5`, `tests/colored_acceptance_simd_o7`, `tests/sdf_collision`, `narrowphase/box_box` | a value change may cross one | never by widening the tolerance in the same commit as the value change | — |

**How a reviewer tells a legitimate re-bless from a masked regression.** Five checks, in order,
each of which the record shows has been skipped at least once:

1. The value-changing commit **declares itself** as a solver-math change in its message and
   touches no pinned constant. A commit that changes arithmetic *and* a golden in one diff is
   rejected on shape alone.
2. On that commit, every **self-comparison** gate (worker-count, run-to-run, SIMD-vs-scalar) is
   green **without any edit** — they need no constant, so a cross-worker race or a mismatched
   fusion site shows up there as red, not as "needs a re-bless". A red self-comparison is never
   answered by re-blessing.
3. The re-bless commit contains **only** the new literal(s) and the captured generator output
   pasted into the commit message. The reviewer re-runs the generator on the value-changing commit
   and confirms the printed value equals the literal in the re-bless commit.
4. Every tolerance gate is green **without a tolerance edit**. A tolerance that has to widen is a
   separate, reviewed change carrying its own measured justification.
5. The plan documents that pin an acceptance number are updated in the same change set, with the
   old and the new number both visible (the memory record's "repairing doc-rot is as error-prone
   as writing it" rule: `git log -G` on the number, not `-S`).

---

## Documentation that this decision leaves stale, and the correction each needs

The policy is unchanged; several of its stated reasons are now measured false, and a false reason
is what the next reader will act on. This pass writes only under `docs/physics/`; the orchestrator
schedules the rest.

| Document · section | Stale text | Correction |
|---|---|---|
| `docs/OPTIMIZATION-PLAN-PHYSICS.md` · "Decision 4: SIMD = width-only, determinism-SAFE, gated on coloring, scalar oracle mandatory" | "The kernels use explicit `a*b + c`. A measurable perf cost ACCEPTED for reproducibility" | No measurable cost on the colored solve (0.98–1.01×); 1.16× forgone on `refresh_inertia`, worth 0.26–0.35 % of a step; fusing the scalar reference would cost 5–9 %. Point to this directory. |
| same section | "`mul_add` lowers to one rounding on FMA-capable CPUs but two without — same source, different bits per target" | `mul_add` is specified as IEEE 754 `fusedMultiplyAdd` (one rounding) and lowers to the hardware instruction on every target this engine builds for (`x86-64-v3`). On a target without the instruction it lowers to the platform libm's `fmaf`, whose correct rounding is that libm's promise, not `rustc`'s — not a shipped configuration here, so nothing rests on it. The hazard is identity with the split oracle, not per-target divergence. |
| same document · "Context and constraints", the determinism-boundary bullet | "no FMA contraction (`mul_add` forbidden in the deterministic path)" | Keep the rule; add that the path is the three crates named in the decision file and that it is gated by the core-wide census, not by prose. |
| `docs/RESEARCH-FAST-MATH.md` · "Open questions for the architect" | "FMA contraction: forbid `mul_add` in the deterministic path entirely … accepting the perf cost … (Recommended yes.)" | Answered: yes, and the perf cost is zero on the solve and 0.26–0.35 % of a step on the inertia refresh. Link here. |
| `solver/simd.rs` module doc · "Determinism is the load-bearing constraint" and `sdf_simd.rs` module doc · "No-FMA / no-approx invariant" | "The two differ by a single ULP on FMA-capable CPUs and would diverge per target" | A fused op is deterministic on every target; it is a different number from the split oracle, and the gate compares against the oracle. |
| `.cargo/config.toml` · the ISA-baseline comment | Names the two per-file censuses as "what keeps the determinism contract load-bearing now" | Both do run, locally and in CI; what they do not do is scan the scalar oracles the SIMD files must match. Name the core-wide census as the gate over the whole core and the per-file pair as the local, cheaper layer. (Revision 2's claim that one of them never ran in CI was false on this tree and is withdrawn.) |
| `tests/isa_baseline_census.rs` · the `fma` entry's reason string | "the physics kernels … are written `mul_add`-free and two source censuses enforce that" | Once the core-wide census lands, point at `docs/physics/` rather than a count of censuses, so the entry does not go stale the next time a gate is added. |
| `.github/workflows/ci.yml` | — | **No change.** Revision 2 scheduled a flag fix here; on this tree the flag is already restated in every physics-building job's `RUSTFLAGS` (commit `cced895a`) and guarded by the root census. The `CARGO_TARGET_…_RUSTFLAGS` route revision 2 recommended is withdrawn: applied to the `force-alloc-panic` job it would have *removed* the restatement that job relies on. |
| `boyko_math` crate header, `soft/*` module docs, `contact.rs`, `colored.rs` | "no `mul_add`/FMA" stated in prose | Keep the prose; it becomes the map of a gate rather than the gate, once the core-wide census lands. For `boyko_math` add the boundary rule from the decision file (section "The rule outside the deterministic core"): fused variants for render or scene are written in the consumer crate, never here. |
