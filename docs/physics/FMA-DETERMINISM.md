# FMA and determinism in `boyko_physics` — the decision

**Status:** decision document, 2026-09-02, revision 3 (after the second architecture critique;
every blocking and non-blocking remark is addressed in place and listed in section 10). Design
pass over a read-only checkout of `feat/threadpool-ke16`; no code in `crates/` was touched.
Companion files:

- [FMA-DETERMINISM-MEASUREMENTS.md](FMA-DETERMINISM-MEASUREMENTS.md) — every number below with
  its method, spread, the CPU it was taken on, the disassembly census that backs it (now with
  register widths), and the flag-delivery measurements relabelled for what they actually measured.
- [FMA-DETERMINISM-ORACLES-AND-PRACTICES.md](FMA-DETERMINISM-ORACLES-AND-PRACTICES.md) — the
  oracle inventory (what each gate asserts, whether fusion could break it, and where it runs), what
  other engines do and what transfers, the priced-and-rejected feature scheme, and the re-bless
  list kept on record for the day a fusing change is ever made deliberately.

**Recommendation: `do-not-fuse`.** The deterministic core of the physics crate (the colored
contact solve, the O1 integrate/inertia kernels, the soft-body kernels, the SDF narrowphase, and
the shared scalar leaves `boyko_math` and `boyko_sdf_math`) keeps its split `mul`+`add` form. No
cargo feature, no wrapper, no second numeric configuration. The measured prize on the kernels that
matter is **zero** on the headline kernel (0.98–1.01×), a **regression** on the shipped default
path (0.91–0.95× in a transcription whose mechanism is confirmed on the in-tree kernel), and
**0.26–0.35 % of a step** on the one kernel where fusion pays in isolation. The rest of this file
is the case, with the numbers.

**What revision 3 corrects, and it is the "summary outlives its retraction" class.** Revision 2
built its safety story on a CI that this checkout does not have: it said the SIMD-vs-scalar gates
"do not run in CI" because `ci.yml` set a bare `RUSTFLAGS: -D warnings`. On this tree, commit
`cced895a` (2026-09-02, "build: ship the ISA baseline the project has declared since it began")
already restates `-C target-cpu=x86-64-v3` inside every `RUSTFLAGS` that builds `boyko_physics`,
and `tests/isa_baseline_census.rs` at the workspace root already asserts five features reached
`rustc`, with a size pin and a negative control, on every `--workspace --all-targets` leg. So the
gates run in CI on the AVX2 configuration today, the proposed ISA-baseline test was a weaker
duplicate, and its landing condition could not be executed. Section 5.4 is rewritten from the tree
as it stands. Revision 2 also specified a census needle for methods that do not exist
(`mul_algebraic(`); the real API is `algebraic_mul(` and it was probed on both toolchains on this
box (section 5.3). Neither correction moves the verdict; both would have sent an implementer to
the wrong file.

**The one thing this pass would put in front of the owner ahead of the FMA answer** (section 9):
the bit-exact 8-wide solve and inertia refresh already exist, are gated and tested, and ship
**off** — `PhysicsConfig::simd`, `simd_solve` and `parallel_solve` all default to `false` and no
production site turns them on. That lever is orders of magnitude larger than anything FMA can buy.

---

## 1. The question

The owner asked (verbatim, translated): *"Could the physics be written so it runs faster with FMA
while staying roughly correct? Or at least make it possible to enable that for physics and other
systems with a compile flag?"*

That is two questions, and they get two different answers:

1. **Can it be written?** Yes — proved in code, not argued. With the fusion sites matched between
   the scalar oracle and the 8-wide kernel, the SIMD-vs-scalar bit-identity oracle survives
   exactly: 0 of 29 752 contact impulses differ, 0 of 90 072 inertia-tensor elements differ,
   0 ULP. Explicit `f32::mul_add` / `_mm256_fmadd_ps` is a deterministic, IEEE-754-specified
   operation; it produces the same bits on every thread, every run, and every vendor. Nothing in
   the worker-count oracles can see it.
2. **Does it run faster?** No, not where it counts. The contact-solve loop that the whole O5–O7
   campaign optimised does not speed up at all when 32 % of its vector arithmetic is fused,
   because the loop is latency-bound on the Gauss–Seidel chain and fusion removes only
   multiplies that were already issuing in parallel. The scalar reference — which is what ships
   by default — gets *slower*, because writing `mul_add` blocks LLVM's partial SLP packing of the
   `Vec3` arithmetic.

The compile-flag half of the question rests on a premise that is true in C/C++ and false in Rust:
there is no contraction to switch on. `rustc` never fuses `a * b + c` on its own — re-confirmed
twice on this checkout, once in the scratch probe and once on the shipped binary itself: a release
test executable of `boyko_physics` built at `x86-64-v3` contains **zero** fused or approximate
opcodes in any of its 6 376 symbols (measurements file, section "In-tree confirmation on the
shipped binary"). The only "flag" that could exist is a hand-written wrapper with two lowerings,
Jolt-style, and that is priced and rejected in section 6.

---

## 2. What determinism this engine actually promises, and what is merely an instrument

Three distinct properties travel under the word "determinism" in this crate. They are not
equally exposed to fusion, and only one of them is a product property at all.

| Property | Where stated | Product or instrument? | Does explicit FMA threaten it? |
|---|---|---|---|
| **(i) Run-to-run reproducibility on one binary, one machine** | `solver/colored.rs` module doc ("the same scene yields the same bits every run") | Instrument — it is the precondition the other gates rest on | **No.** A fused op is a pure function of its operands. Only compiler contraction (which Rust does not do) and atomics ever threatened it. |
| **(ii) Independence from worker count** | `colored_tests::parallel_solve_is_bit_identical_across_worker_counts`, `colored_columns_snapshot_is_byte_identical_across_workers_and_runs`, `colored_rigid_scratch_determinism`, `ke16_app1_solve_in_system_bit_identity`, `integrate_determinism` | Instrument — the crate's **race detector** for a lock-free parallel solve that Miri cannot reach (the pool's int-to-ptr is documented Miri-intractable) | **No.** Every worker runs the same ops on disjoint rows; fusion changes the value, never its dependence on thread count. See the two paragraphs on reductions and on the control word below. |
| **(iii) Bit-identity between the SIMD path and the scalar reference** | `solver/simd.rs` module doc, `solve_color_avx2` doc, `sdf_simd.rs` module doc; gates `simd_solve_bits_match_scalar`, `refresh_inertia_simd_bits_match_scalar`, `x8_bits_eq_scalar_bits_widened_proptest`, `tests/simd_o1` | Instrument — the **0 %-gate**: the only way to land a widening or a storage refactor as *provably value-neutral* | **Only if the two paths fuse at different sites.** Preservable (measured 0/29 752) but fragile (one mismatched site out of ~151 per contact produced 2 799/29 752 divergences at up to 256 ULP, in a transcription that looked faithful). |

**Property (ii) covers reassociation as well as fusion.** The separate hazard to worker-count
independence is a reduction whose *order* depends on how many workers took part — a per-chunk
partial sum merged in arrival order, say. None exists on the physics path today; each site was
checked rather than assumed (and re-traced by the critique against the source):

- the broadphase's parallel `emit_passes` writes per-cell disjoint ranges and ends in one serial
  `out.sort_unstable()` over unique `(min, max)` keys (`resources.rs`, the `BroadphaseGrid`
  emit path), so the chunk count is a performance knob only;
- the colored solve touches every dynamic body from exactly one manifold group per colour
  (`solver/colored.rs`, the `is_dynamic_row` argument), so there is no cross-worker accumulation;
- `IslandSleep::end_step` accumulates a per-island **maximum** speed², serially, not a sum
  (`resources.rs`), so it is order-free by construction;
- the soft-rigid reaction (`SoftRigidReaction::accumulate`) runs inside the serial
  `for body in query.iter_mut()` loop of `physics_soft_step_coupled` (`soft/solver.rs`);
- `physics_integrate` is a per-row `par_iter_mut` with no reduction (`systems.rs`);
- no `fetch_add` / `AtomicU*` counter exists anywhere under `boyko_physics/src`.

So the worker-count oracles are self-comparisons of one code path over a fixed operation order,
and neither an explicit FMA nor a future reassociation could make them depend on the worker count
without first introducing a reduction that does not exist.

**Property (ii) also assumes one floating-point control word on every thread that executes a
colour, and nothing in the tree pins or checks it.** A grep over `crates/` for `_mm_getcsr`,
`_mm_setcsr`, `FLUSH_ZERO`, `DENORMALS_ZERO`, `fesetround` finds nothing — the engine never
touches MXCSR. That is the right policy, but it is a policy about *this* code, not about the
process: `solve_color_parallel` solves colours below `MIN_PARALLEL_SLOTS_PER_COLOR` (256 slots)
inline on the calling thread and larger ones on pool workers, and the threshold's doc comment
promises the two are bit-identical. They are, *if* the calling thread and the workers agree on
rounding mode and on FTZ/DAZ. A third-party runtime that sets flush-to-zero on the thread it is
initialised from (some audio and graphics runtimes document exactly this) would make the inline
colours and the worker colours differ on the denormal inputs the adversarial corpora deliberately
contain — a value-visible, worker-count-dependent divergence that no FMA decision creates or
removes. The oracle plan (section 7) therefore adds a **debug-build control-word probe**, placed
outside the census roots because the only non-deprecated way to read MXCSR is inline assembly
(`_mm_getcsr` warns as deprecated on this toolchain and would fail `-D warnings`).

**There is no product requirement behind any of the three.** The engine has no replay system,
no lockstep networking, and no save format that stores solver trajectories — every "lockstep" in
`docs/` is the word in an unrelated sense, and the book's single "replay" is a hypothetical benefit
of the fixed-timestep accumulator (`book/src/app/time.md`), not a feature. All three properties
exist to let the crate be *tested*: (ii) catches cross-worker races that have a value signature,
(iii) lets a refactor claim "nothing changed" and be believed.

What the record says these instruments have and have not caught is in the companion file, section
"What the bit oracles caught". The short form: across O4–O11 and Stage P, both real cross-worker
races (the SP4 pinned-particle race and the O7 gather read) were *value-benign* by construction and
were found by Miri-TB and by adversarial review, not by any value oracle; the one defect the byte
gates did surface (a `cohort_shape` registration-order bug on the P2 path) is of a class a
tolerance test could miss, which is why the byte gates are worth keeping. What they bought beyond
that is the 0 %-gate — P1, P2, O1, O6 and O7 were each landed as provably value-neutral because of
it. That is a refactoring instrument, and its value peaks exactly when a change claims to change
nothing. A fusing solver is not such a change.

**One genuine product-adjacent constraint exists, and it is not in the solver.** `boyko_sdf_math`
is the single frozen leaf shared by the CPU narrowphase and by the GPU golden mirror in
`boyko_rhi_vulkan`; its own header says a reordered FMA "could push a golden past its ±2/255
tolerance — this must NOT be 'cleaned up'". The GPU end's fusion policy belongs to `dxc` and the
driver, not to the engine. That leaf, and `sdf_simd.rs` which widens it op-for-op, is the one place
where fusion would change a contract with something the engine does not control. Note that the SDF
x8 arm is `cfg`-selected, not flag-selected (`systems.rs`, the narrowphase dispatch), so it is the
one AVX2 kernel a default-configuration shipped binary actually executes.

**The boundary of the deterministic core, stated precisely so the census in section 5 can gate
it:** every `.rs` file under `crates/boyko_physics/src`, every file under `crates/boyko_math/src`
(the physics `Vec3`/`Mat3`/`Quat` are *re-exports* of `boyko_math` — the scalar oracle for
`refresh_inertia` is literally `boyko_math::Mat3::from_quat` and `impl Mul for Mat3`, and
`boyko_math`'s own header promises "no `f32::mul_add`/FMA … anywhere in this crate"), and every
file under `crates/boyko_sdf_math/src`. `boyko_math` is *also* a render and scene dependency; that
does not move it out of the core, it means fused arithmetic for render lives **outside**
`boyko_math` (section 5.2).

---

## 3. The measured prize, per kernel

All timings: scratch crate outside the repository, on an **AMD Ryzen 9 5900HS (Zen 3, 8 cores /
16 threads, 3.3 GHz base)**, `rustc 1.97.1` `stable-x86_64-pc-windows-gnu`,
`-C target-cpu=x86-64-v3`, `opt-level=3`, `lto=fat`, `codegen-units=1`; kernels transcribed
op-for-op from `solver/simd.rs` and `solver/colored.rs`; sizes from `benches/parallel_solve.rs`
(10 011 bodies, 29 752 contacts). Ratios are `split_median / fused_median`; > 1 means fusion is
faster. Eight invocations over ~20 minutes under concurrent-agent contention; per-row spreads in
the measurements file. The CPU matters: on Zen 3 `vfmadd213ps` has latency 4 while `vaddps` and
`vmulps` have latency 3 (uops.info), so on a serial accumulate chain the split form is *strictly*
faster here and a wash on Skylake/Ice Lake — a reader on another core should re-take the
latency-bound rows before treating them as universal.

| Kernel (file · function) | Fusible sites | Share of FP ops fused | Measured split/fused | Verdict |
|---|---|---|---|---|
| **Colored contact solve, 8-wide** — `colored.rs::solve_color_avx2` over the `simd.rs` x8 helpers (`cross8`, `dot8`, `mat3mulvec8`, `pointvel_x8`, `effective_mass_x8`, `apply_impulse_blend_x8`) | 145 per 8 contacts (counted in the compiled loop: 3 per cross, 2 per dot, 28 per effective mass, 12 per impulse apply) | 32 % of vector FP arithmetic; 12.6 % of all loop instructions; spills fall 51→40 stores | **0.98–1.01×** (8 runs; median 1.00) | **No prize.** Latency-bound on the serial Gauss–Seidel chain; `mul→add→add` and `mul→fma→fma` have equal depth. Confirmed by an L1-resident-gather variant (memory is ~2.5 % of the loop) that still shows 0.97–1.01×. |
| **Colored contact solve, scalar reference** — `colored.rs::solve_color` over `contact.rs::{effective_mass, point_velocity, apply_impulse}` and `boyko_math` | 145 per contact (identical sites) | LLVM SLP-packs the split form's `Vec3` arithmetic (transcription: 57 `vmulps`/30 `vaddps`; **in-tree `solve_color`, default release profile: 93 `vmulps`/38 `vaddps`/24 `vsubps`, every one of them `xmm`** — the symbol contains no `ymm` op at all, so none of it is an inlined AVX2 callee; the scalar/SIMD fork is in `solve_color_dispatch`) next to 92/49/28 scalar ops; the fused transcription has **zero packed ops** (145 `vfmadd*ss`) | **0.91–0.95×** in the transcription (7 clean runs of 8; the two arms never overlap) | **Regression.** The mechanism — partial packing that `mul_add` defeats — is confirmed on the in-tree kernel by disassembly; the *magnitude* is the transcription's, because the in-tree fused variant cannot be built without editing `crates/`. This is the path that ships: `PhysicsConfig::simd` and `simd_solve` both default to `false`, with no production call site setting them. |
| **Inertia refresh, 8-wide** — `simd.rs::refresh_inertia_avx2` (`quat_to_mat3_x8` + `mat3_mul_x8` ×2) | 36 per 8 bodies (2 per 3-term dot × 18 dots) | ~34 % of vector FP ops | **1.14–1.23×** (8 runs; median 1.16) | **Real, in isolation.** 18 independent dot products, no chain to hide behind. |
| **Inertia refresh, scalar reference** — `simd.rs::refresh_inertia_scalar` = `Mat3::from_quat`, `Mat3 * Mat3`, `transpose` in `boyko_math` | 36 per body | Transcription: not packed in either form (63 `vmulss`/60 `vaddss` split; 36 `vfmadd*ss` fused). **In-tree, the split form *is* partially packed** (7 `vmulps`/6 `vaddps`/2 `vsubps` on `xmm` **plus 3 `vmulps`/2 `vaddps` on `ymm`**, beside 21/18/4 scalar ops), which the transcription lacked | **1.14–1.16×** (3 runs, 1.136–1.161) — an **upper bound** for the in-tree kernel | **Real in the transcription; likely smaller in-tree**, because the same packing that `mul_add` defeats in the solve is present here too. |
| **Inertia refresh, as a share of a step** — 4 refreshes vs 12 solve passes per step at the defaults (`substeps = 4`, `relax_iterations = 2`) | — | inertia is 2.2–2.6 % of solve+inertia time | fusing it saves **0.26–0.35 % of the step** (both paths; less if the in-tree scalar bound above binds) | **Below the noise floor** of every in-tree criterion bench (their run-to-run spreads are several percent). Counting broadphase, narrowphase, graph build, gather and apply shrinks the share further. |
| **Gravity / position integrate** — `simd.rs::apply_gravity_avx2`, `position_integrate_block_x8` | 3 of 6 / ~18 of 54 | 50 % / ~35 % | not timed | Streaming pass; memory-bound. `position_integrate` already ships **scalar** by measured O1 choice (the AoS gather overwhelmed the arithmetic — the same mechanism that flattens the solve). |
| **SDF narrowphase** — `sdf_simd.rs::{sd_sphere_x8, sd_box_x8, smin_x8, sdf_edit_list_x8}` | 2 of 10 / 2 of 19 / ~2 of 11 / 0 in combine and clamp | ~10–15 % (in-tree `sdf_edit_list_x8`: 24 `vmulps`/16 `vaddps`/31 `vsubps` against 15 `vmaxps`, 6 `vminps`, 4 `vsqrtps`, 3 `vdivps`, all `ymm`) | not timed — argued from the op census; the ceiling is too low to matter | **Worst prize, worst cost.** Dominated by `max`/`min`/`sub`/`sqrt`/`blend`; fusing it means fusing the frozen `boyko_sdf_math` leaf and putting the CPU↔GPU ±2/255 goldens in play. |
| **Soft-body kernels** — `soft/{solver, colored, coupling, self_collision, collide}.rs` | not counted per site | small: XPBD projection is `sqrt`/`div`-dominated per constraint | **not timed — argued from the op shape, not measured** | No prize expected; the SP4 serial golden pins them; a value change buys a re-bless for nothing. |
| **Box–box SAT narrowphase** — `narrowphase/box_box.rs` | 28 `*` in 959 lines | small; branch-bound | **not timed — argued from the op census, not measured** | Not worth widening for; its `to_bits` manifold gate and 1e-4 tolerances would need a re-bless for nothing. |

**How much the value moves if the solver fuses** (the size of the re-bless, measured): 7 029 of
29 752 converged normal impulses change (23.6 %), maximum gap **6 848 ULP** — not a last-bit
wobble; the friction cone's `sqrt` and the `1/k` reciprocal amplify one rounding. 66 168 of 90 072
inertia-tensor elements change (73 %), including sign flips on near-zero off-diagonals. Every pinned
hash moves; every tight tolerance has to be re-argued rather than assumed.

---

## 4. What other projects do, and what transfers

Condensed from the companion file's section "Practices survey". The one-line version of each:

- **Box2D 3.1** bans FMA for the whole library with `-ffp-contract=off` and promises three levels
  of determinism by default. The ban targets **compiler contraction**, which Clang does by default
  (`-ffp-contract=on`) and GCC does across statements (`-ffp-contract=fast` for C++). Rust has no
  such default, so the ban has no Rust equivalent and no Rust need. What transfers: the
  bit-array-OR merge for worker-count independence (our oracles already assert the same property),
  and the finding that *transcendentals*, not arithmetic, are the cross-platform hazard.
- **Jolt** is the one engine that ships explicit FMA as its fast tier (`JPH_USE_FMADD`, one wrapper
  `Vec4::sFusedMultiplyAdd` with two lowerings, a CMake option to turn it off for cross-platform
  determinism at a measured ~8 %). It does **not** maintain any fused-equals-unfused bit-identity;
  its fast tier promises only same-binary determinism. It is the exact shape of the owner's
  second question, and the exact shape rejected in section 6 — for us it would buy ≤ 0.35 %.
- **Rapier** (Rust) promises cross-platform bit-level determinism behind `enhanced-determinism` and
  spends **none** of that budget on FMA — the feature is a dependency swap (`simba/libm_force`) for
  the transcendental library, plus a lane-width incompatibility (`simd8`). A production Rust
  physics engine with determinism as its selling point does not mention FMA anywhere.
- **PhysX, Bullet, Unity DOTS, Factorio, the RTS-lockstep literature, Bruce Dawson's analysis:**
  none of them treats explicit FMA as a same-binary hazard. Dawson's same-binary/same-processor
  list is "altered FPU settings, uninitialised variables, and non-floating-point sources" — FMA is
  not on it, and "altered FPU settings" is the control-word assumption made explicit in section 2.
  Unity's only fully general cross-vendor answer is soft-float, i.e. the opposite of this engine's
  constraints.
- **Rust itself:** `f32::mul_add` is IEEE `fusedMultiplyAdd`, "guaranteed not to change". Two
  adjacent features are *not* safe for a bit oracle and must be gated against now, and revision 3
  probed their real spellings on this box rather than quoting them: the **`float_algebraic`
  family is spelled `algebraic_add`, `algebraic_sub`, `algebraic_mul`, `algebraic_div`,
  `algebraic_rem`** (prefix form) — behind the unstable `float_algebraic` gate on `rustc 1.97.1`,
  and compiling with **no feature gate on `nightly 1.100.0 (2026-08-20)`**, i.e. stabilised and
  one release away from this toolchain; the spelling revision 2 wrote into its needle
  (`mul_algebraic(`) does not exist on either. `mul_add_relaxed` (`llvm.fmuladd`, "1 or 2
  roundings, nondeterministically") exists on **neither** toolchain yet; its needle is a forward
  guard. The existing censuses ban the literal `mul_add(` — that does **not** match
  `mul_add_relaxed(` (the substring is `mul_add_` followed by `relaxed(`) and does not match any
  `algebraic_*(` at all. Section 5.3 closes both.
- **uops.info:** in throughput-bound batched code FMA is ~2× on the FP ports; in a latency-bound
  serial accumulator the split form is *strictly faster* on Haswell, Zen 2/3/4 and Alder Lake-P
  (`vaddps` latency 2–3 vs `vfmadd` 4–5). Which regime a kernel is in is per-loop, which is why the
  measured answer above is per-kernel and not uniformly positive.

---

## 5. The scheme: what stays split, the gate that keeps both sides split, and where the gates run

### 5.1 What stays split, and why, per kernel

| Kernel | Stays split because |
|---|---|
| Colored solve (scalar and 8-wide) | No measured prize on the 8-wide path; a measured regression on the scalar path that ships, with the mechanism confirmed in-tree; the largest re-bless surface in the crate. |
| Inertia refresh (scalar and 8-wide) | A real 1.16× on the 8-wide kernel (an upper bound of 1.16× on the scalar one) that is 0.26–0.35 % of a step — invisible to every in-tree bench, and the price is a re-bless of `bodytype_determinism_golden::GOLDEN`, a rewrite of both censuses into site-pairing, and a permanent two-file hand-maintained invariant across `boyko_math` and `simd.rs`. |
| Gravity / position integrate | Memory-bound; `position_integrate` already ships scalar by measurement. |
| Soft-body kernels (`soft/{solver, colored, coupling, self_collision, collide}.rs`) | Not timed; argued from the op shape (`sqrt`/`div`-dominated projection). Prose-only no-FMA rule today; the SP4 serial golden pins them. |
| SDF narrowphase and `boyko_sdf_math` | ~10–15 % of ops fusible; the leaf is frozen for a CPU↔GPU parity contract the engine does not own. |
| Box–box narrowphase | Not timed; argued from the op census (28 `*` in 959 branch-bound lines). |

### 5.2 The rule outside the deterministic core — fused arithmetic lives where it is used

Outside the three crates named in section 2 there is **no rule against `mul_add` and nothing to
enable**: `boyko_demo` already uses it (`sim/systems/common.rs`, `benches/gpu_pack.rs`), and so
does a `boyko_app` test. Rust fuses only where a human writes `mul_add`, so "enabling FMA for other
systems" is a per-site decision made with a profile, exactly like any other micro-optimisation.

The one rule is about the boundary:

- **`boyko_math` contains no fused or approximate arithmetic. Not as a new method, not behind a
  feature, not in a marked block.** Its own header already promises exactly this ("no
  `f32::mul_add`/FMA … anywhere in this crate"), the census keeps it literally true, and the
  physics oracle is one `pub use` away from every op in it — a fused sibling there would be a
  single import from silently redefining `refresh_inertia`'s reference.
- **A render or scene system that wants a fused variant writes it in its own crate** — a free
  function or an extension trait over the `boyko_math` types in `boyko_render` (or wherever the
  profile that justified it lives), outside the three census roots. Fused arithmetic is thereby
  always one crate boundary away from the deterministic core, and the census stays
  exemption-free, which is the property that keeps it from being weakened the first time it fires.
- Why not a named exemption (a `boyko_math::fast` module the census skips)? Because the tree's own
  record says an exemption is what a gate becomes the moment it is inconvenient (the a0336bd9
  release-vacuity fix, the `hwrt` arm in `docs/OPEN-QUESTIONS.md`), and because the demand is
  hypothetical: outside `boyko_demo` and one `boyko_app` test there is no fused call site in the
  workspace today. A rule with zero exemptions is cheaper to keep than a rule with one.
- **`boyko_scene` is deliberately outside the roots, and the next reader should not "fix" that.**
  It is a physics dependency on the *data* path: `sync_transform_to_body` copies kinematic and
  static poses from `Transform` into `RigidBody` before integrate. A fused op added there would
  change physics *inputs* deterministically and move the goldens without any census firing. That is
  a value change of the kind any upstream edit can make (a scene loader, an animation system), not
  a determinism or SIMD-vs-scalar hazard, and the goldens are the right instrument for it. Widening
  the census to every upstream crate would turn a boundary into a crusade.

### 5.3 The gate — a census over the whole core, not two files, plus a crate-local AVX2-arm guard

Today's enforcement is two per-file censuses, `solver::simd::tests::solver_simd_has_no_fma_or_approx_callsites`
and `sdf_simd::o9_kernel_tests::sdf_simd_has_no_fma_or_approx_callsites`. They are well built
(comment-skipping, runtime-assembled needles so they cannot flag themselves, a non-vacuity
witness) and both run in CI on this tree (section 5.4). What they guard is **two files**, while
the scalar oracles those files must match — `contact.rs`, `colored.rs`, `soft/*`, `boyko_math`,
`boyko_sdf_math` — carry the same rule in doc comments only. A `mul_add` written into
`boyko_math::Mat3::mul` today *is* caught — by `bodytype_determinism_golden` (the hash moves
because the value moves) and by `refresh_inertia_simd_bits_match_scalar` (the 8-wide kernel no
longer matches its oracle). What those catches lack is **locality and timing**: the golden says
"something changed somewhere in 90 steps of physics" and invites a re-bless; the bit gate names a
kernel, not a line. The census names the line and fires before any golden is touched.

**What the census does and does not prove, stated so it is not over-read.** It keeps *both sides
fusion-free*; it does not keep them *in step*. Claim (iii) — the 8-wide kernel equals the scalar
oracle bit for bit — is proved only by the differential bit gates, and there is a divergence class
the census cannot see that only they cover: `_mm256_max_ps`/`_mm256_min_ps` return the second
operand on a NaN and on a signed-zero tie, while `f32::max`/`f32::min` are IEEE `maxNum`/`minNum`;
the in-tree disassembly shows 2 `vmaxss` in `solve_color` and 2 `vmaxps` in `solve_color_avx2`.
The adversarial differential tests (`degenerate_lane_differential_test_1d`, the `-0.0`/denormal
corpora) are the cover for that, and they are what a widening must keep green. The census is the
cheap, early, line-numbered layer; the bit gates are the proof.

The implementer adds **one test file**, `crates/boyko_physics/tests/deterministic_core_fp_census.rs`,
with two tests:

**Test 1 — `deterministic_core_has_no_fused_or_approx_callsites`.**

- **File set:** every `*.rs` under `CARGO_MANIFEST_DIR/src`, `CARGO_MANIFEST_DIR/../boyko_math/src`
  and `CARGO_MANIFEST_DIR/../boyko_sdf_math/src`. Cross-crate scanning from a physics test has
  precedent: the W4 gate in `sdf_simd.rs` already walks another crate's tree with a recursive
  `scan_dir` that skips `target`.
- **Line filter:** non-comment lines only — `//`, `//!`, `///` prefixes skipped after
  `trim_start`, exactly as the per-file censuses do today. **Accepted, and stated so it is not
  discovered:** block comments (`/* … */`) and string literals are *not* skipped. A needle inside
  a `/* */` block or a `"…"` literal is flagged. Today's core has no such text; a future one is
  rewritten as a line comment, not exempted.
- **Needles, assembled at runtime from fragments** (`"algebraic_" + stem + "("`, `width + stem`,
  and so on) so the test never flags its own text. Every spelling below was **probed on this box**
  (`rustc 1.97.1` stable and `nightly 1.100.0`) rather than quoted; the probe recipe is in the
  measurements file, section "API spelling probes":
  - *safe-Rust methods:* `mul_add(`; `mul_add_relaxed(` (a forward guard — exists on neither
    toolchain today; the `mul_add(` needle verifiably does not match it); and the five
    `algebraic_{add, sub, mul, div, rem}(` (unstable on 1.97.1, ungated on nightly 1.100 —
    stabilised);
  - *nightly intrinsics (`core::intrinsics`):* `_algebraic(` (the `f{add,sub,mul,div,rem}_algebraic`
    spellings), `_fast(` (the `f*_fast` spellings — **stated as intentional** that any future
    identifier ending in `_fast(` collides; zero hits today; rename, never exempt),
    `fmaf16(`, `fmaf32(`, `fmaf64(`, `fmaf128(`, `fmuladdf16(`, `fmuladdf32(`, `fmuladdf64(`,
    `fmuladdf128(`, `simd_fma(`, `simd_relaxed_fma(`;
  - *foreign / libm spellings:* `fmaf(` and `fma(` — the second over-matches any identifier ending
    in `fma(`; zero hits today; same rule, rename;
  - *the x86 intrinsic family, at three widths, matched by prefix so the suffix does not matter:*
    `{_mm512_, _mm256_, _mm_}` × `{fmadd, fmsub, fnmadd, fnmsub, fmaddsub, fmsubadd, rsqrt, rcp}`
    — 24 needles that catch `_mm256_fmadd_ps(`, `_mm_fmadd_ss(`, `_mm512_fmadd_round_ps(`,
    `_mm512_rsqrt14_ps(`, `_mm512_rcp28_ps(` and a bare `use core::arch::x86_64::_mm256_fmadd_ps;`
    alike. (`CLAUDE.md` names AVX-512 as opt-in via `cfg(target_feature)`, so a `_mm512_` tail
    handler is a plausible future edit; this is the row that catches it.)
  - *inline assembly:* `asm!(` — one needle that covers `asm!`, `global_asm!` and `naked_asm!`,
    because a `vfmadd`/`vrsqrt` mnemonic inside an `asm!` block is invisible to every intrinsic
    needle. Zero hits in the three roots today; the control-word probe of section 7 is placed in
    `boyko_utils` precisely so this needle can stay absolute.
- **Self-reference:** the core-wide scan covers `solver/simd.rs` and `sdf_simd.rs`, which contain
  the two per-file censuses. Their needle tables are fragment-assembled today (`"mul_add"` +
  `"("`, `widths` × `stems` + `suffix`), so they are not flagged; the same convention is mandatory
  for any future needle added to them. The core-wide census's own file is under `tests/`, outside
  the scanned roots.
- **Non-vacuity, in both directions:** assert the scan saw all three crate roots and a file count
  above a floor; assert it saw `_mm256_mul_ps(` in `solver/simd.rs` and `impl Mul for Mat3` in
  `boyko_math/src/mat.rs` (proof it read the right text); assert that **one synthetic line per
  needle**, built by concatenation and fed to the same matcher, **is** flagged (proof the matcher
  is not dead — the clause the existing censuses lack, and the clause that would have caught
  revision 2's misspelled needle); and assert that a synthetic fragment-only line
  (`"mul_add", "("`) is **not** flagged (proof the fragment convention still protects the per-file
  censuses).
- **Exemptions:** none inside the core. A future kernel that genuinely needs an approximation lives
  outside these three crates or behind a new decision that rewrites this document.
- **The two existing per-file censuses stay unchanged.** They are a strict subset of this one,
  cheaper, and already in the crate's unit-test binary; both run under the same
  `cargo test -p boyko-physics` as the new file. Aligning their needle tables is optional and is
  not part of this change — that keeps this pass out of two files a concurrent agent is editing.

**Test 2 — `avx2_gated_kernels_and_their_gates_are_present_in_this_build`.** Revision 2's
`isa_baseline_reaches_this_test_build` is **deleted**: it was a two-feature subset of the root
census `tests/isa_baseline_census.rs` (five features, a size pin, a negative control), and its
landing condition ("together with the CI flag fix, after one deliberate red") referred to a fix
that had already landed. One ground survives, and the replacement is justified on it alone: the
mandated invocation **`cargo test -p boyko-physics` never runs the root package's census**, so a
developer or agent whose shell carries a `RUSTFLAGS` (`--emit=asm`, `--cfg loom`, a stray
`-D warnings`) gets a lib binary with **127 tests instead of 136**, two integration files compiled
out, and a green run (measurements file, section "Flag delivery"). The crate-local guard is:

- `#[cfg(all(target_arch = "x86_64", not(miri)))]`, asserting **one** predicate —
  `cfg!(target_feature = "avx2")` — because that is the cfg this crate's own `#[cfg]` arms are
  keyed on, not an ISA level. It does **not** restate the five-feature list; its message names the
  root census as the authority on the baseline and lists what this crate loses without the arm
  (`mod sdf_simd` and its five tests, four `solver::colored::tests` gates,
  `tests/colored_simd_parallel_o7.rs`, `tests/colored_acceptance_simd_o7.rs`, and five
  `solver::simd::tests` plus `tests/simd_o1` degraded to scalar-equals-scalar).
- It is green today locally and in CI. Its ability to fail is demonstrated, not assumed, by the
  measured route `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C target-feature=-avx2,-fma"`
  (the target-scoped form merges with the config and keeps the machine's linker fix), which must
  be run once, red, before the test lands.
- A `compile_error!` was considered and rejected: it would take the whole test binary down,
  including Test 1, which is the "one red hides the rest" shape the `--no-fail-fast` finding of
  2026-08-10 already documented.

### 5.4 Where the gates run — the tree as it stands

Revision 2's account of CI was wrong for this checkout and is withdrawn in full. Verified by
reading `.github/workflows/ci.yml` and `tests/isa_baseline_census.rs` on `feat/threadpool-ke16`:

- **Every CI job that builds `boyko_physics` carries the ISA baseline.** The workflow-wide
  environment is `RUSTFLAGS: "-D warnings -C target-cpu=x86-64-v3"`, with a comment stating the
  replace-not-append mechanism and the day it was measured; the `force-alloc-panic` job restates
  it in its own `RUSTFLAGS` because a job-level env replaces the workflow-level one. The `check`,
  `test` (debug and release), `clippy`, `bench-compile` and `force-alloc-panic` jobs therefore all
  build the **AVX2 configuration** — the same one a developer builds with `.cargo/config.toml` in
  force.
- **The Miri job deliberately does not carry the flag, and does not need it:** it keeps
  `RUSTFLAGS: "-D warnings"` with a rationale, and its sweep is scoped to `boyko-ecs`,
  `boyko-utils`, `boyko-threadpool`, `boyko-serialize`, `boyko-math`, `boyko_sdf_math` and
  `boyko_image` — it never builds `boyko_physics`, and the two census roots it does build carry
  no `cfg(target_feature)` arm.
- **The flag is guarded, not trusted.** `tests/isa_baseline_census.rs` in the root package
  (`"."` is a `default-member`, so every `--workspace --all-targets` leg runs it) asserts that
  `avx2`, `fma`, `bmi2`, `lzcnt` and `f16c` reached `rustc`, pins the list's length at five, and
  evaluates the same predicate against `avx512f` as a negative control. A future job that sets its
  own `RUSTFLAGS` without the ISA flag is a named red test, not a smaller engine.
- **What the flag-delivery measurements in the measurements file establish**, now correctly
  labelled: the *mechanism* (a `RUSTFLAGS` environment variable replaces every
  `[target.*].rustflags`; the target-scoped `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` form merges) and the
  *size of the loss* if the flag is ever dropped (136 → 127 unit tests; the nine named; two
  integration files; six gates degraded to tautologies). The row that revision 2 captioned "exactly
  CI's value" measured `RUSTFLAGS="-D warnings"` alone — the value CI set **before** `cced895a`,
  not the value it sets now. The root census exists because of that measurement; the measurement
  is not a description of CI.

Consequence for this design's safety story: "both sides stay split, and the bit gates keep the
8-wide kernels equal to their oracles" holds in CI today on the configuration that ships. What
remains, and what the two new tests add, is the `-p` invocation gap (Test 2) and the scalar
oracles' prose-only rule (Test 1). Two corrections still fall out of the tree as it stands:

- **`.cargo/config.toml`'s ISA-baseline comment** names the two per-file censuses as "what keeps
  the determinism contract load-bearing now". Both do run in CI; what is missing is that neither
  scans the scalar oracles the SIMD files must match. Correct it to name the core-wide census
  (section 5.3) as the gate over the whole core and the per-file pair as the local, cheaper layer.
- **`tests/isa_baseline_census.rs`'s `fma` entry** says the physics kernels are "written
  `mul_add`-free and two source censuses enforce that"; once the core-wide census lands, "three"
  — or better, point at this directory rather than a count.

### 5.5 The docs that must change, even though the policy does not

The policy survives; two of its stated *reasons* do not, and a stale reason is the seed of the
next wrong decision. The orchestrator schedules these (this pass writes only under
`docs/physics/`):

- `docs/OPTIMIZATION-PLAN-PHYSICS.md`, section "Decision 4: SIMD = width-only, determinism-SAFE,
  gated on coloring, scalar oracle mandatory": *"A measurable perf cost ACCEPTED for
  reproducibility"* is refuted. Replace with: no measurable cost on the colored solve (0.98–1.01×);
  a 1.16× kernel-local gain forgone on `refresh_inertia`, worth 0.26–0.35 % of a step; fusing the
  scalar reference would cost 5–9 % *in a transcription of the kernel whose packing mechanism is
  confirmed in-tree*. Same section: *"`mul_add` lowers to one rounding on FMA-capable CPUs but two
  without — same source, different bits per target"* is the C/C++ contraction story, not Rust's;
  `mul_add` is one rounding per IEEE 754 on every target where the platform `fmaf` is conforming
  (a software `fma()` is what a CPU without the instruction gets, and its correctness is the
  libm's, not `rustc`'s). The hazard is identity with the split oracle, not per-target divergence.
- The module docs of `solver/simd.rs` ("Determinism is the load-bearing constraint") and
  `sdf_simd.rs` ("No-FMA / no-approx invariant") say a fused op "would diverge per target". Reword
  to the true statement: a fused op is deterministic; it is a *different number* from the split
  oracle, and the gate compares against the oracle.
- `docs/RESEARCH-FAST-MATH.md`, section "Open questions for the architect": the FMA question is
  answered here; point to this file.
- `.cargo/config.toml` and `tests/isa_baseline_census.rs` as in section 5.4.

---

## 6. The compile-flag option, priced and rejected

The owner's second sentence deserves a direct answer, not a deflection. If a feature were built it
would look like this — the names are given so the rejection is of a concrete thing:

- `boyko_physics` features `fp-split` (default) and `fp-fused`, mutually exclusive by
  `compile_error!` when both are set; a build with neither behaves as `fp-split`.
- One wrapper pair, `mla(a, b, c)` / `mls(a, b, c)` in `boyko_math`, with the split body under
  `fp-split` and `mul_add` under `fp-fused`; every fusible site in the scalar oracles **and** the
  x8 helpers routed through it so both arms fuse at the same places by construction.
- Both censuses made configuration-aware: under `fp-split` they ban the fused family and
  `mul_add(` everywhere in the core; under `fp-fused` they ban the fused family and `mul_add(`
  everywhere *except* inside the wrapper's own body, and additionally assert that every
  fusible site goes through the wrapper (a second census, over the pattern `* ` followed by `+ `
  on one expression, which is not decidable by grep and would have to be a review rule).
- Two blessed values for every pinned golden (`bodytype_determinism_golden::GOLDEN`,
  `soft_colored_sp4_baseline::SP4_SERIAL_GOLDEN`), selected by `cfg`.
- CI gains a third and fourth `cargo test -p boyko-physics --no-default-features --features fp-fused --all-targets`
  leg (debug and release), because a fast path that only compiles is an untested path — this
  repository's own record on `hwrt` (`docs/OPEN-QUESTIONS.md`), on the a0336bd9 release-vacuity
  fix, and on the 2026-08-10 `--no-fail-fast` finding says exactly what happens to the arm CI does
  not run. As `ci.yml` stands those legs would inherit the workflow-wide `RUSTFLAGS` with the ISA
  flag restated, so they would build the fused AVX2 arm; a job that gave them their own
  `RUSTFLAGS` would have to restate the flag, and the root census would say so if it did not.

What that buys: **0.26–0.35 % of a physics step**, on a configuration nobody runs by default,
while every to_bits oracle in the crate must hold in two numerically different engines and every
tolerance acceptance gate in `colored_acceptance_o5.rs` / `colored_acceptance_simd_o7.rs` /
`sdf_collision.rs` must be re-argued for the fused arm. The cost is the largest of the three
options (no fusion / unconditional fusion / feature) and the prize is the same as unconditional
fusion's. Rejected on the number.

---

## 7. Oracle plan under `do-not-fuse`

Nothing in the existing oracle set changes meaning. The plan is additive. **"Runs in"** has two
honest values on this tree: *local* (a developer's `cargo test -p boyko-physics` with
`.cargo/config.toml` in force — the AVX2 configuration) and *CI* (the workflow as it stands after
`cced895a` — also the AVX2 configuration, on every job that builds the crate; the Miri job does not
build it).

| Oracle | Runs in | Change |
|---|---|---|
| Worker-count and run-to-run self-comparisons (`colored_tests::parallel_solve_is_bit_identical_across_worker_counts`, `colored_columns_snapshot_is_byte_identical_across_workers_and_runs`, `tests/colored_rigid_scratch_determinism`, `tests/ke16_app1_solve_in_system_bit_identity`, `tests/integrate_determinism`) | local and CI | None. |
| `tests/colored_simd_parallel_o7` (the {parallel × simd × N-worker} self-comparison) | local and CI (crate-level `avx2` gate is satisfied on both) | None. |
| SIMD-vs-scalar bit gates in `solver::colored::tests` (`simd_solve_bits_match_scalar`, `cone_adversarial_differential_test_1c`, `degenerate_lane_differential_test_1d`, `cohort_shape_proptest_bit_exact_and_non_vacuous`) | local and CI | None; their meaning is preserved because neither side fuses. They, not the census, are the proof of claim (iii), including the `max`/`min` NaN-semantics class. |
| `solver::simd::tests::{refresh_inertia, apply_gravity, position_integrate}_simd_bits_match_scalar`, `degenerate_quat_lane_matches_scalar`, `adversarial_inputs_simd_bits_match_scalar`; `tests/simd_o1` | local and CI, SIMD versus scalar | None. Test 2 of 5.3 makes the scalar-equals-scalar configuration a named red on a `-p` run that has lost the arm. |
| `sdf_simd::o9_kernel_tests::x8_bits_eq_scalar_bits_widened_proptest`, `assert_lane_bit_exact`, `x8_inert_lanes_do_not_leak` (against the frozen `boyko_sdf_math` leaf) | local and CI | None; the leaf stays frozen. |
| Pinned goldens (`tests/bodytype_determinism_golden::GOLDEN`, `tests/soft_colored_sp4_baseline::SP4_SERIAL_GOLDEN`) | local and CI | None — no value moves. Re-bless procedure recorded in the companion file for any future deliberate solver-math change. |
| `solver::simd::tests::solver_simd_has_no_fma_or_approx_callsites`, `sdf_simd::o9_kernel_tests::sdf_simd_has_no_fma_or_approx_callsites` | local and CI | Keep unchanged; the core-wide census is their superset. |
| Root `tests/isa_baseline_census.rs` (five features, size pin, negative control) | CI (every `--workspace --all-targets` leg) and a local `cargo test --workspace`; **not** a local `cargo test -p boyko-physics` | None. It is the authority on the baseline; Test 2 defers to it. |
| **New** `tests/deterministic_core_fp_census::deterministic_core_has_no_fused_or_approx_callsites` (section 5.3, Test 1) | local and CI, unconditionally — no `cfg` | Create. Turns the prose-only rule in `contact.rs`, `colored.rs`, `soft/*`, `boyko_math` and `boyko_sdf_math` into a check with a line number, with the probed needle spellings and the per-needle positive witness. |
| **New** `tests/deterministic_core_fp_census::avx2_gated_kernels_and_their_gates_are_present_in_this_build` (section 5.3, Test 2) | local and CI: green; red on any `-p` run whose environment dropped the arm | Create, after being seen red once via the measured `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C target-feature=-avx2,-fma"` route. |
| **New, recommended** — debug-build floating-point control-word probe | local and CI debug legs (compiled out in release) | Add `boyko_utils::fp_env::x86_control_word_is_default() -> bool`: `#[cfg(all(target_arch = "x86_64", not(miri)))]` reads MXCSR with a read-only `stmxcsr` in `asm!` (`_mm_getcsr` is deprecated on this toolchain and fails `-D warnings`), masks the six sticky exception-flag bits, and compares the rest to the IEEE default `0x1F80` (round-to-nearest, all exceptions masked, FTZ and DAZ clear); every other cfg returns `true`. `boyko_physics` already depends on `boyko_utils`. Assert it with `debug_assert!` at the entry of `ColoredSoftStepSolver::step` (the calling thread) and at the top of each closure `solve_color_parallel` spawns into `pool.scope` (the workers). Cost: one `stmxcsr` per colour chunk per pass in debug builds, zero in release. It lives outside the census roots so the `asm!(` needle stays absolute, and outside the Miri sweep's reach because Miri cannot execute inline assembly. Makes the assumption of section 2 a check rather than a sentence. |
| Tolerance acceptance gates (`tests/colored_acceptance_o5`, `tests/colored_acceptance_simd_o7`, `tests/sdf_collision`, `narrowphase/box_box` manifold `to_bits` + 1e-4 anchors) | local and CI | None now. Under any future fusion they are re-run and re-argued (value moves up to 6 848 ULP), never widened in the same commit as the value change. |
| `colored_tests::colored_columns_snapshot_matches_pre_p2_vec_baseline` | local (native only); CI: runs and passes vacuously | Pre-existing vacuity unrelated to FMA: it reads `D:/tmp/p2_baseline_columns.txt` and silently returns when absent, so it cannot fail on any machine but the one that captured it. Flagged for the owner (section 9). |

---

## 8. Costs

- **Engineering:** one new test file (the core-wide census and the crate-local AVX2-arm guard,
  ~200 lines in the house style of the existing censuses, with the probed needle table), the
  optional control-word probe (~30 lines in `boyko_utils` plus two `debug_assert!` sites), and the
  doc corrections in sections 5.4 and 5.5. No kernel edits; no re-bless; no `ci.yml` edit.
- **Forgone:** 1.16× on `refresh_inertia`, worth 0.26–0.35 % of a step; nothing on the solve;
  nothing on the SDF.
- **Kept:** every bit oracle at its current meaning and exercised by CI on the shipped
  configuration; the 0 %-gate available for the next width or storage refactor;
  `boyko_sdf_math`'s CPU↔GPU contract untouched; a single numeric configuration of the engine to
  test; `boyko_math` 100 % split with an exemption-free census.
- **The re-bless list is empty for this decision** but is kept on record in the companion file,
  section "Re-bless list", so that a future deliberate solver-math change (the only legitimate
  reason to move `GOLDEN`) has the inventory ready and a reviewer can tell a re-bless from a
  masked regression.

---

## 9. The owner's remaining calls

Genuine values or scope questions only; every performance and architecture fork above was decided
here with its number, including the boundary rule in 5.2, the deletion of the duplicate ISA test
in 5.3, and the placement of the control-word probe in 7.

1. **The lever the owner is actually asking about is not FMA — it is the SIMD configuration that
   ships off.** `PhysicsConfig::simd`, `simd_solve` and `parallel_solve` all default to `false`
   (`resources.rs`), `PhysicsPlugin` inserts `..PhysicsConfig::default()` and overrides none of
   them, and the only `simd_solve = true` sites in the workspace are tests. Production therefore
   runs the single-threaded **scalar** solve. By this pass's own numbers the scalar inertia refresh
   is 3.8× slower than the already bit-exact 8-wide kernel, and the scalar rank body is ~4.5×
   slower than the 8-wide one in the transcription; O7's in-tree acceptance target for the 8-wide
   solve was ≥ 1.8× atop the parallel scaling. FMA's ceiling is 0.26–0.35 % of a step. **Scope
   call:** make `simd = true` and `simd_solve = true` the `PhysicsPlugin` default? The design's
   recommendation is **yes for those two** — they are bit-identical to the scalar path by the very
   oracles this document keeps, on every CI leg — and **not yet for `parallel_solve`**, which is
   bounded by KE16 defect A (a `scope` issued from a worker is drained serially by the joiner;
   `benches/ke16_solve_in_system.rs` exists to measure exactly that route) until that campaign
   lands, and which is the path on which the O7 review's cross-worker gather read was found. This
   is a change to another campaign's shipped default, so it is the owner's, not this pass's.
2. **`colored_columns_snapshot_matches_pre_p2_vec_baseline` reads a file on `D:/tmp` and passes
   silently without it.** Scope call: delete it (the {1,N} and run-to-run byte gates above it cover
   the property), or check the captured baseline into the repository so the test can actually
   fail elsewhere. Unrelated to FMA; surfaced because the oracle inventory found it.
3. **Does the owner want the "roughly correct" (`±`) framing recorded anywhere as a future
   option?** The design's answer is that no such mode is needed because there is no prize to spend
   the bit gates on; if the owner disagrees on *values* — i.e. wants a non-bit-exact physics mode
   for its own sake — that is a new campaign, not a variant of this one, and it should start from
   the rejected scheme in section 6 with the prize re-measured on a kernel that has one.

---

## 10. What changed in revision 3, against the second critique

| Remark | Disposition |
|---|---|
| **Blocking 1** — sections 5.3/5.4/7/8 described a CI without the ISA flag and a `tests/` tree without the root census; Test 2 duplicated a five-feature gate with two features; its landing condition was unexecutable; the recommended `CARGO_TARGET_…_RUSTFLAGS` route would have *removed* the restatement the `force-alloc-panic` job relies on | **Accepted; re-based on the tree.** `ci.yml` line by line and `tests/isa_baseline_census.rs` in full were re-read; section 5.4 now states what runs where and why; every `runs_in` is "local and CI"; the old Test 2 is deleted and replaced by a one-predicate crate-local guard justified only on the `-p` invocation gap, deferring to the root census by name, with an executable red-first route; the `ci.yml` direction is withdrawn; the measurements file relabels the `RUSTFLAGS="-D warnings"` row as pre-`cced895a`, and its test-list rows as the size of the hazard rather than CI's state; the `.cargo/config.toml` correction is kept with its false half ("one of them never runs in CI") removed. |
| **Blocking 2** — the algebraic needle was spelled for methods that do not exist (`mul_algebraic(`), so the real `algebraic_mul(` would pass | **Accepted; probed, not quoted.** Both toolchains on this box were compiled against: `algebraic_{add,sub,mul,div,rem}` exist (unstable behind `float_algebraic` on 1.97.1; ungated on nightly 1.100.0), `mul_algebraic` does not, `mul_add_relaxed` does not on either. The needle table is rewritten with the real spellings, a per-needle synthetic positive witness is mandatory, and the two prose sites are corrected (sections 4, 5.3). |
| Non-blocking — further spellings: nightly `fmaf32(`/`fmuladdf32(`/`simd_fma(`, `libm::fmaf(`, the `_mm512_` width, `asm!` mnemonics | **Added** (section 5.3): all ten intrinsic spellings probed on nightly; `fma(`/`fmaf(`; the intrinsic family widened to three widths and matched by prefix so `_round_`/`14`/`28` suffixes are caught; `asm!(` banned outright in the core, which is why the control-word probe lives in `boyko_utils`. |
| Non-blocking — MXCSR assumed equal across threads; zero sites pin or check it; inline-vs-worker split makes a stray FTZ value-visible | **Stated and gated** (sections 2 and 7): the grep result recorded; the assumption written under property (ii); a debug-build `stmxcsr` probe specified with its home, its two assert sites, its Miri and census interactions, and its cost. |
| Non-blocking — the real lever (SIMD/parallel configuration shipped off) buried as a timing footnote | **Promoted to owner call 1** with the numbers, a recommendation for the two SIMD flags, and the KE16 bound on `parallel_solve`. |
| Non-blocking — "the census … runs in every configuration" read as proof of claim (iii); `max`/`min` NaN semantics | **Reworded** (section 5.3, "What the census does and does not prove"; section 7 row 3). |
| Non-blocking — soft-body and box–box rows argued, not timed | **Labelled** "not timed — argued from the op census" in sections 3 and 5.1. |
| Non-blocking — packed-op counts without register width | **Re-censused with widths** from the surviving release disassembly: `solve_color` is 100 % `xmm` (no inlined AVX2 callee possible), `refresh_inertia_scalar` is `xmm` plus a few `ymm` bundles, the 8-wide kernels are 100 % `ymm`; recorded in section 3 and the measurements file. |
| Non-blocking — `boyko_scene` is a physics input path outside the roots | **Stated** (section 5.2) with the reason it must stay outside. |
