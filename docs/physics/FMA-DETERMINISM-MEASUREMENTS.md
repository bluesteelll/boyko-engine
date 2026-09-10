# FMA and determinism — measurements

Companion to [FMA-DETERMINISM.md](FMA-DETERMINISM.md) (the decision). This file holds every
number that document cites, with the method that produced it, the spread across runs, and the
disassembly census that backs the claims about what the compiler emitted. Nothing here was taken
from a note; every figure was produced on this checkout's toolchain on 2026-09-02. Revision 2
added three sections that were measured on the tree itself rather than on a transcription: the
opcode census of a release test binary of `boyko_physics`, the `rustc` argv under three
flag-delivery routes, and the unit-test list with and without the AVX2 arm. Revision 3 adds the
register width to the opcode census, **relabels the flag-delivery rows for what they measured**
(the mechanism, and the size of the loss if the ISA flag is dropped — not the state of CI, which
has carried the flag since commit `cced895a`), and adds the API-spelling probes that fix the
census needle table. No timing was re-taken; no timing was disputed.

---

## Method

**Where.** A scratch crate (`fmaprobe`) outside the repository, in the session scratchpad. No
file under `crates/` was written; `solver/simd.rs` and `sdf_simd.rs` were being edited by a
concurrent agent throughout and were only read.

**Machine.** AMD Ryzen 9 5900HS (Zen 3, 8 cores / 16 threads, 3.3 GHz base), Windows 11. The
core matters for the latency-bound rows: on Zen 3, uops.info gives `VFMADD213PS ymm` latency 4 /
throughput 0.50, `VMULPS ymm` 3 / 0.50, `VADDPS ymm` 3 / 0.50 — so a serial `acc = acc + a*b`
chain pays 3 cycles per link split and 4 fused, and the "no prize" result on the rank kernel is
what that arithmetic predicts *on this core*. On Skylake/Ice Lake (4 / 4 / 4) it would be a wash;
on Alder Lake-P (`vaddps` 2 vs `vfmadd` 4) the split form's edge is larger. Re-take rows A and D
before quoting them for another microarchitecture.

**Toolchain and flags.** `rustc 1.97.1 (8bab26f4f 2026-07-14)`, `cargo 1.97.1 (c980f4866
2026-06-30)`, `stable-x86_64-pc-windows-gnu` (the reason the scratch crate pins its own
`rust-toolchain.toml` was recorded here as "the box's default toolchain is MSVC 1.92.0 with no
linker" — ⚠ FALSE as of 2026-09-10: no 1.92.0 exists on the box, `stable-x86_64-pc-windows-msvc` is
`rustc 1.98.1 (48a229cea 2026-09-01)` and links, and it is what this tree's Windows recipes spell
since that date — though the rustup DEFAULT host is still `x86_64-pc-windows-gnu` on 2026-09-10,
so a bare `cargo` here still selects gnu. The
pin is still what makes these rows attributable; the rows themselves are gnu-1.97.1 numbers and are
not re-taken here). `.cargo/config.toml` in the
scratch crate: `-C target-cpu=x86-64-v3` — the same ISA baseline the worktree's
`.cargo/config.toml` now sets per x86_64 target. Profile: `opt-level = 3`, `lto = "fat"`,
`codegen-units = 1`, `debug = false`. **This is not the shipped profile:** the workspace has no
`[profile.release]`, so a `cargo build --release` of the tree uses `codegen-units = 16` and no
LTO. The in-tree census below was taken under the shipped profile; the timings were not.

**What was transcribed.** Three kernels, each in a *split* arm (separate `mul` then `add`/`sub`,
as the tree has it) and a *fused* arm (`_mm256_fmadd_ps`/`_mm256_fmsub_ps` in the 8-wide code,
`f32::mul_add` in the scalar code), both arms otherwise op-for-op identical:

- **A / C / D — the colored contact-solve rank body**: one full normal solve + 2-DOF friction
  cone per contact, over a 20-column contact SoA with the O7 per-cohort scalar gather of 8 lanes ×
  2 bodies × 13 fields transposed to stack SoA. Transcribed from `solve_color` /
  `solve_color_avx2` in `solver/colored.rs` and the `x8` helpers in `solver/simd.rs`
  (`cross8`, `dot8`, `mat3mulvec8`, `pointvel_x8`, `effective_mass_x8`,
  `apply_impulse_blend_x8`). A = 8-wide with the real strided gather; C = the scalar reference;
  D = A with every lane's body indices remapped into the first 16 bodies so the gather is
  L1-resident.
- **B / E — the inertia refresh** `R · I⁻¹_local · Rᵀ`: quaternion-to-matrix then two 3×3
  products, left-associated, 18 three-term dots per body. B = 8-wide (`quat_to_mat3_x8`,
  `mat3_mul_x8`); E = the scalar reference, which in the tree is `boyko_math::Mat3::from_quat`,
  `impl Mul for Mat3` (`a.x*b0 + a.y*b1 + a.z*b2`, left to right) and `transpose`. In both arms
  the `from_quat` diagonals `1 - 2*(..)` stay split, so the fused arms differ from the split arms
  only at the 36 dot-product sites — the differential is apples-to-apples.

**Sizes.** From `benches/parallel_solve.rs`: pyramid base 141 → 10 011 bodies, 29 752 contacts.

**Timing protocol.** Per invocation: 3 warm-up passes, then 61 timed passes; the median is the
row's value, `min` and `p90` printed alongside as the contamination signal. Eight invocations in
all: five by the ground-truth pass (10:25–10:30) and three by this pass (10:39:52, 10:42:06,
10:45:35), each separated by minutes, under acknowledged concurrent-agent CPU contention. A row
whose `p90` exceeds its median by more than ~10 % is reported and marked contaminated, not
dropped silently.

**Bit comparison.** `f32::to_bits` equality per output; "ULP gap" is the absolute difference of
the raw bit patterns, which over-states a gap that crosses zero (see the inertia value-move row).

---

## Re-confirmation: rustc does not contract

The brief's premise — that `rustc` never fuses an explicit `a * b + c` — was re-confirmed by
disassembling the built probe (`llvm-objdump -d` from the pinned toolchain) rather than by reading
the note in `.cargo/config.toml`. Per-symbol opcode census, FMA present in the ISA for every row:

| Symbol | `vfmadd*ps` | `vfmadd*ss` | `vmulps` | `vaddps` | `vsubps` | `vmulss` | `vaddss` | `vsubss` |
|---|---|---|---|---|---|---|---|---|
| `rank_split` (A, split) | **0** | 0 | 254 | 118 | 81 | 0 | 7 | 0 |
| `rank_fused` (A, fused) | **145** | 0 | 109 | 35 | 19 | 0 | 7 | 0 |
| `scalar_split` (C, split) | 0 | **0** | 57 | 30 | 17 | 141 | 54 | 42 |
| `scalar_fused` (C, fused) | 0 | **145** | 0 | 0 | 0 | 109 | 35 | 9 |
| inertia x8 split (B, inlined closure) | **0** | 0 | 63 | 60 | 6 | 0 | 7 | 0 |
| inertia x8 fused (B, inlined closure) | **36** | 0 | 27 | 24 | 6 | 0 | 7 | 0 |
| `scalar_inertia_split` (E, split) | 0 | **0** | 0 | 0 | 0 | 63 | 60 | 6 |
| `scalar_inertia_fused` (E, fused) | 0 | **36** | 0 | 0 | 0 | 27 | 24 | 6 |

Three facts read straight off the table:

1. **No split arm contains a single fused instruction** despite `+fma` being enabled. The old
   `compile_error!` guards were rejecting a build that could not have fused anything.
2. **Every fused arm contains exactly the count that was written**: 145 per 8 contacts in the rank
   body, 36 per 8 bodies (2 per three-term dot × 18 dots) in the inertia refresh. Fusion in Rust
   is a per-site, human-authored, countable decision.
3. **The scalar solve is partially packed in its split form** (57 `vmulps`, 30 `vaddps`,
   17 `vsubps` — packed ops in a scalar loop: LLVM's SLP vectoriser bundles the three `Vec3`
   components of a `dot`/`cross`/`mul_vec` into one `xmm` op) and **not at all in its fused form**
   (zero packed ops). `f32::mul_add` defeats that packing here. This is the mechanism behind the
   scalar regression in the timing table, and it is **confirmed on the in-tree kernel** in the
   next section. The scalar inertia refresh is not packed in either form *in the transcription*;
   in-tree it is (next section), so row E over-states the in-tree gain.

---

## In-tree confirmation on the shipped binary

Added in revision 2, because a transcription is evidence about the transcription. A release test
executable of `boyko_physics` was built **in the worktree, under the shipped profile, with
`.cargo/config.toml` in force** (`cargo test --release -p boyko-physics --test simd_o1 --no-run`,
into a separate `CARGO_TARGET_DIR` so no concurrent agent's fingerprints were disturbed), then
disassembled with the toolchain's `llvm-objdump -d` and censused per symbol (6 376 symbols):

| In-tree symbol (release, `x86-64-v3`, `codegen-units = 16`, no LTO) | fused / approx ops | `vmulps` | `vaddps` | `vsubps` | `vmulss` | `vaddss` | `vsubss` | other |
|---|---|---|---|---|---|---|---|---|
| `ColoredSoftStepSolver::solve_color` (the scalar rank body, the bit oracle; 901 instructions) | **0** | **93** (all `xmm`) | **38** (all `xmm`) | **24** (all `xmm`) | 92 | 49 | 28 | 1 `vsqrtss`, 4 `vdivss`, 2 `vmaxss`; **no `ymm` operand anywhere in the symbol** |
| `ColoredSoftStepSolver::solve_color_avx2` (the 8-wide cohort kernel; 1 710 instructions) | **0** | 127 (`ymm`) | 66 (`ymm`) | 42 (`ymm`) | 0 | 0 | 0 | 1 `vsqrtps`, 1 `vdivps`, 2 `vmaxps` |
| `simd::effective_mass_x8` / `apply_impulse_blend_x8` / `pointvel_x8` | 0 / 0 / 0 | 48 / 18 / 6 | 19 / 12 / 3 | 12 / 3 / 3 | — | — | — | 1 `vdivps` in `effective_mass_x8` |
| `simd::refresh_inertia_avx2` (618 instructions) | **0** | 63 (`ymm`) | 51 (`ymm`) | 6 (`ymm`) | 0 | 0 | 0 | — |
| `simd::refresh_inertia_scalar` (the O1 oracle = `boyko_math::Mat3` ops; 168 instructions) | **0** | **10** (7 `xmm` + 3 `ymm`) | **8** (6 `xmm` + 2 `ymm`) | **2** (`xmm`) | 21 | 18 | 4 | — |
| `sdf_simd::sdf_edit_list_x8` (210 instructions) | **0** | 24 (`ymm`) | 16 (`ymm`) | 31 (`ymm`) | 0 | 0 | 0 | 4 `vsqrtps`, 3 `vdivps`, 15 `vmaxps`, 6 `vminps` |
| **every symbol in the executable** (std, proptest, criterion, the ECS, the pool included) | **0** `vfmadd*`, `vfmsub*`, `vfnmadd*`, `vfnmsub*`, `vrsqrt*`, `vrcp*` | | | | | | | |

**Register width, recorded in revision 3 because the reading depends on it.** `solve_color` is a
scalar function that never calls the AVX2 kernel (the fork is `solve_color_dispatch`), so its
packed ops can only be SLP bundles of the three `Vec3` components — and the width column proves
it: every packed op in the symbol is `xmm`, and no `ymm` operand appears at all, so none of the 93
`vmulps` is an inlined AVX2 callee. `refresh_inertia_scalar` carries a few `ymm` bundles on top of
its `xmm` ones (LLVM packed eight of the matrix product's terms), which is the same mechanism one
level wider. A reader re-running the census on a build where `solve_color_dispatch` inlines
differently should keep the width column; a `ymm` op in `solve_color` would mean the fork moved.

Four facts read straight off the table:

1. **The shipped configuration emits no fused and no approximate op anywhere**, not only in the
   censused files — with FMA in the ISA. (This read "the **two** censused files" when it was
   written on 2026-09-02 and there were two; there are four since 2026-09-03, covering every file
   in the crate that holds an `_mm256_*` call site. The binary-level fact this item states is
   unchanged and is what makes the point: it was true of the whole crate even where no census yet
   looked.) This is the in-tree form of "rustc does not
   contract", on the real profile, and it is also the binary-level check that the source censuses
   approximate: any `vfmadd` in a `boyko_physics`/`boyko_math`/`boyko_sdf_math` symbol of a
   default-flag build is a violation of the rule this design keeps.
2. **The in-tree scalar `solve_color` is partially packed** (93 / 38 / 24 packed beside 92 / 49 /
   28 scalar), through the `solve_view()` accessors and all — the same mechanism the transcription
   showed with 57 / 30 / 17. So the regression mechanism for fusing the shipped default path is a
   property of the real kernel, not of the transcription's loop shape. What remains attributed to
   the transcription is the **magnitude** (5–9 %): the in-tree fused variant cannot be built
   without editing `crates/`, which this pass may not do.
3. **The in-tree scalar `refresh_inertia_scalar` is also partially packed** (10 / 8 / 2 beside
   21 / 18 / 4). The transcription's row E arm had no packed ops in either form, so the 1.14–1.16×
   measured there is an **upper bound** for the in-tree scalar refresh; fusing it in-tree would
   also defeat this packing and could land anywhere down to a wash, by the same mechanism as the
   solve.
4. **`solve_color_avx2` and `sdf_edit_list_x8` are what their censuses say they are:** all
   multiply and add, no fused op, exact `vsqrtps`/`vdivps`, no `vrsqrt`/`vrcp`.

The census script (per-symbol opcode counts over `llvm-objdump -d --no-show-raw-insn`) is a
40-line Python file in the session scratchpad; the same counts can be reproduced from any release
test binary of the crate with the toolchain's `llvm-objdump`.

---

## Flag delivery: what reaches `rustc`

**What this section measures, stated in revision 3 because revision 2 mis-captioned it:** the
*mechanism* by which a `RUSTFLAGS` environment variable displaces `.cargo/config.toml`'s target
rustflags, and the *size of the loss* if the ISA flag is ever dropped. It is **not** a description
of CI on this tree: `.github/workflows/ci.yml` has carried `-C target-cpu=x86-64-v3` inside every
physics-building job's `RUSTFLAGS` since commit `cced895a` (2026-09-02), and the root package's
`tests/isa_baseline_census.rs` fails any `--workspace --all-targets` leg that loses it. The row
below captioned "the value CI set" measured the value CI set *before* that commit. The decision
file's section "Where the gates run — the tree as it stands" has the current picture.

`cargo build -p boyko-math -v` in the worktree, the `Running \`rustc …\`` line
inspected for the flags of interest, `cargo 1.97.1`, fresh `CARGO_TARGET_DIR` per row. Two config
files are in play on this box: the worktree's `.cargo/config.toml` (`[target.x86_64-pc-windows-gnu]
rustflags = ["-C", "target-cpu=x86-64-v3"]`) and the machine-local `~/.cargo/config.toml`
(`[target.x86_64-pc-windows-gnu] rustflags = ["-Cdlltool=…", "-L …", "-Clink-arg=…"]`, a
documented fix for the mingw `dlltool`/`ld` on this machine).

| Environment | Flags present in the `rustc` argv | Reading |
|---|---|---|
| nothing set | `-C target-cpu=x86-64-v3`, `-Cdlltool=…`, `-Clink-arg=…` | Both config files' `[target.*].rustflags` lists are **joined**. |
| `RUSTFLAGS="-D warnings"` — the value `.github/workflows/ci.yml` set workflow-wide **before** `cced895a`; it now sets `-D warnings -C target-cpu=x86-64-v3` | `-D warnings` and **nothing else** | `RUSTFLAGS` **replaces** every `[target.*].rustflags` list in every config file. The ISA flag is gone; so is the machine's own linker fix — which is why every `RUSTFLAGS`-bearing build of the physics test binary on this box died at `error calling dlltool 'dlltool.exe': program not found` (with `-D warnings`, with `-C target-cpu=x86-64-v3 -D warnings`, with `-C target-cpu=x86-64`, and with the empty string — five attempts, same failure), while the same build with no `RUSTFLAGS` succeeded in 36 s. Same mechanism, seen from the other side. |
| `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-D warnings"` — the target-scoped environment form of the same config key | `-C target-cpu=x86-64-v3`, `-Cdlltool=…`, `-Clink-arg=…`, `-D warnings` | The target-scoped environment form **merges** with the config files' lists, config first, environment appended. |
| `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C target-feature=-avx2,-fma"` | `-Cdlltool=…`, `-C target-cpu=x86-64-v3`, `-C target-feature=-avx2,-fma` (in that order) | Used below to produce the non-AVX2 configuration through a route that keeps the machine's linker fix. |

**The test list, with and without the AVX2 arm.** `cargo test -p boyko-physics --lib -- --list`,
same toolchain, same profile (`test`), only the ISA differing (the last row above versus nothing
set):

| Configuration | Unit tests in the `boyko_physics` lib binary (as measured 2026-09-02) |
|---|---|
| `x86-64-v3` (the config file's baseline; what a developer runs) | **136** |
| `x86-64-v3` with `avx2` and `fma` subtracted (what any `RUSTFLAGS` that omits the ISA flag produces — a developer's `--emit=asm` shell, a `--cfg loom` run, or CI before `cced895a`) | **127** |

> ⚠ **Both counts are as-measured on 2026-09-02 and the crate has gained tests since**; they are
> kept with their date rather than refreshed, because the quantity this table is *for* is the
> **difference** — the size of the loss when the ISA flag is dropped — and re-taking one number
> without the other would make the gap a fiction. At least the two censuses added on 2026-09-03
> (`systems_has_no_fma_or_approx_callsites`, `colored_has_no_fma_or_approx_callsites`) belong to
> both columns, since neither is ISA-gated, so they raise both counts equally and leave the
> difference where it is. **Measured 2026-09-03 on the baseline row only:**
> `cargo test -p boyko-physics --lib -- --test-threads=1 has_no_fma_or_approx` reports
> `running 4 tests` with `134 filtered out`, i.e. **138** unit tests — exactly the 136 above plus
> the two new censuses, which is the arithmetic this note predicts. The second row was NOT
> re-taken, so the gap of 9 is still 2026-09-02's. Anyone re-taking these must re-take **both rows
> in one sitting**, on the two routes named above.

The nine that vanish: `sdf_simd::o9_kernel_tests::{sdf_simd_has_no_fma_or_approx_callsites,
w4_rhi_vulkan_has_no_x8, x8_bits_eq_scalar_bits_widened_proptest, x8_empty_list_is_far_all_lanes,
x8_inert_lanes_do_not_leak}` (the whole module is `cfg`-gated on `avx2` in `lib.rs`) and
`solver::colored::tests::{simd_solve_bits_match_scalar, cone_adversarial_differential_test_1c,
degenerate_lane_differential_test_1d, cohort_shape_proptest_bit_exact_and_non_vacuous}`. Present
in both lists but comparing scalar to scalar without the arm:
`solver::simd::tests::{refresh_inertia, apply_gravity, position_integrate}_simd_bits_match_scalar`,
`degenerate_quat_lane_matches_scalar`, `adversarial_inputs_simd_bits_match_scalar`. Present in
both and meaningful in both: **three of the four no-FMA censuses** —
`solver_simd_has_no_fma_or_approx_callsites` and, since 2026-09-03,
`systems::o9_manifold_tests::systems_has_no_fma_or_approx_callsites` and
`solver::colored::tests::colored_has_no_fma_or_approx_callsites` (both under a plain
`#[cfg(test)]`, no ISA gate) — plus every worker-count and run-to-run self-comparison. The fourth,
`sdf_simd_has_no_fma_or_approx_callsites`, is in the vanishing list above with the rest of its
module. Integration-test files are not in the lib list; two of them carry
a crate-level `avx2` gate and vanish the same way (`tests/colored_simd_parallel_o7.rs`,
`tests/colored_acceptance_simd_o7.rs`), and `tests/simd_o1.rs` compiles but, per its own header,
"trivially holds" without the arm.

---

## API spelling probes — what the census needles must actually match

Added in revision 3 after the critique found that revision 2's `_algebraic(` needle was spelled
for methods that do not exist. Rather than quote a tracking issue, each spelling was compiled on
this box: a one-file probe under `rustc --crate-type=lib --emit=metadata`, once per toolchain,
reading the error list. Toolchains: `rustc 1.97.1 (8bab26f4f 2026-07-14)` stable-gnu (the tree's)
and `rustc 1.100.0-nightly (8925ea358 2026-08-20)` nightly-gnu (installed on this box).

| Spelling | `1.97.1` stable | nightly `1.100.0` | Needle in the core-wide census |
|---|---|---|---|
| `f32::algebraic_add`, `algebraic_sub`, `algebraic_mul`, `algebraic_div`, `algebraic_rem` | exist, **unstable** (`E0658: use of unstable library feature float_algebraic`, issue #136469) | **compile with no feature gate** — stabilised | `algebraic_` + stem + `(` — five needles |
| `f32::mul_algebraic` (revision 2's spelling) | `E0599: no method named mul_algebraic` | `E0599` | none — does not exist |
| `f32::mul_add_relaxed` | `E0599: no method named mul_add_relaxed` | `E0599` — not implemented yet | `mul_add_relaxed(` — a forward guard |
| `f32::mul_add` | stable | stable | `mul_add(` (verified: the substring does not occur in `mul_add_relaxed(`) |
| `core::intrinsics::{fmaf32, fmaf64, fmuladdf16, fmuladdf32}` | — (nightly only) | resolve under `#![feature(core_intrinsics)]` | `fmaf16(`, `fmaf32(`, `fmaf64(`, `fmaf128(`, `fmuladdf16(`, `fmuladdf32(`, `fmuladdf64(`, `fmuladdf128(` |
| `core::intrinsics::{fadd_algebraic, fmul_algebraic}` | — | resolve | `_algebraic(` |
| `core::intrinsics::{fadd_fast, fmul_fast}` | — | resolve | `_fast(` |
| `core::intrinsics::simd::{simd_fma, simd_relaxed_fma}` | — | resolve | `simd_fma(`, `simd_relaxed_fma(` |
| `core::arch::x86_64::_mm_getcsr` | compiles with `warning: use of deprecated function … use inline assembly instead` — fails `-D warnings` | same | not a needle; the reason the control-word probe is inline `asm!` and lives outside the roots |

The scratch probe files are in the session scratchpad (`apiprobe/`); the recipe is the two
`rustc` invocations above with `RUSTUP_TOOLCHAIN` set per row.

---

## The differential proof: matched fusion preserves SIMD-vs-scalar bit-identity

The hypothesis the brief asked to prove or refute in code: explicit FMA preserves the
SIMD-vs-scalar oracle if and only if both paths fuse at the same sites.

| Comparison | Differing outputs | Max gap |
|---|---|---|
| A 8-wide split vs C scalar split (transcription control) | 0 / 29 752 | 0 |
| **A 8-wide fused vs C scalar fused** | **0 / 29 752** | **0** |
| B 8-wide split vs E scalar split, all 9 tensor elements (control) | 0 / 90 072 | 0 |
| **B 8-wide fused vs E scalar fused, all 9 tensor elements** | **0 / 90 072** | **0** |

Deterministic; identical on all eight executions. The control rows prove the transcriptions are
faithful; the bold rows prove the hypothesis in the affirmative for both kernels.

**The refutation of "just write `mul_add` everywhere"**, from the ground-truth pass's first
transcription: it fused ONE site differently — the SIMD side computed the normal-impulse delta as
`fmsub(-impulse_coeff, lambda_n, -t)` where the scalar side computed
`fmsub((-mass_coeff * m_eff), (vn + bias), impulse_coeff * lambda_n)`. Both are "the same
expression". Result: **2 799 / 29 752 impulses differ, up to 256 ULP**, from one site in ~151 per
contact, in a kernel that still converged plausibly. The property is preservable, not automatic,
and its failure is quiet. That is why the design's gate (FMA-DETERMINISM.md, section "The gate — a
census over the whole core, not the AVX2 files") bans fusion on *both* sides rather than trusting them
to agree.

---

## How far the value moves if a kernel fuses

Split arm vs fused arm of the *same* path — the size of any re-bless, measured rather than guessed:

| Kernel | Outputs that change | Max gap |
|---|---|---|
| Contact solve (both A-vs-A and C-vs-C) | 7 029 / 29 752 converged normal impulses (23.6 %) | **6 848 ULP** |
| Inertia refresh (both B-vs-B and E-vs-E) | 66 168 / 90 072 tensor elements (73.5 %) | raw-bit gap 3 015 591 970 — i.e. **sign flips**: near-zero off-diagonal elements land on opposite sides of zero |

The 23.6 % is the uninteresting number (most impulses clamp at 0 and are unchanged); the 6 848-ULP
maximum is the interesting one, because it says the divergence is not a last-bit wobble on the
contacts that matter — the friction cone's `sqrt` and the `1/k` reciprocal amplify one rounding
difference. Any pinned hash moves; any tight tolerance must be re-argued, not assumed to hold.

---

## Timings

Ratios are `split_median / fused_median`; a value above 1 means the fused arm is faster.
Ground-truth runs are labelled G1–G5 (10:25–10:30), this pass's runs P1–P3 (10:39:52, 10:42:06,
10:45:35).

### A — colored solve rank body, 8-wide, real gather (29 752 contacts per pass)

| Run | split median µs | fused median µs | ratio | note |
|---|---|---|---|---|
| G2–G5 | 365.3–380.0 | 371.4–379.0 | 0.9931, 1.0106, 0.9836, 0.9928 | G2 contaminated (p90 555.7 on one row), reported not relied on; G1 predates the dlam-site fix and is excluded |
| P1 | 371.8 | 370.3 | 1.0041 | clean |
| P2 | 379.6 | 377.7 | 1.0050 | clean |
| P3 | 379.3 | 377.2 | 1.0056 | clean |

**Range 0.984–1.011, median ≈ 1.00. No prize.** This despite the fused build removing 145 of 453
vector FP arithmetic instructions (32 %), 165 of 1 309 total loop instructions (12.6 %), and
reducing register spills from 51 stores / 58 reloads to 40 / 35 — all verified in the disassembly.

### D — same kernel, gather made L1-resident

| Run | split median µs | fused median µs | ratio |
|---|---|---|---|
| G (two runs) | ~357 | ~357 | 0.9719, 1.0031 |
| P1 | 366.5 | 366.0 | 1.0014 |
| P2 | 371.5 | 368.7 | 1.0076 |
| P3 | 369.1 | 372.1 | 0.9919 |

**Range 0.972–1.008.** Removing the gather's memory cost moved the split kernel by only ~2.5 %
and did not make the fused arm faster. The loop is neither memory- nor throughput-bound; it is
**latency-bound** on the serial Gauss–Seidel chain (`m_eff → vn → dlambda → impulse → apply →
next rank`). In a chained dot written `mla(a2,b2, mla(a1,b1, mul(a0,b0)))` the dependency depth is
`mul→add→add` split and `mul→fma→fma` fused — identical. FMA deletes the independent multiplies,
which were already issuing on another port for free.

### B — inertia refresh, 8-wide (10 011 bodies per pass)

| Run | split median µs | fused median µs | ratio |
|---|---|---|---|
| G1–G5 | 27.9–32.4 | 24.5–26.3 | 1.1774, 1.2319, 1.1641, 1.1457, 1.1388 |
| P1 | 29.4 | 25.5 | 1.1529 |
| P2 | 29.9 | 25.8 | 1.1589 |
| P3 | 30.0 | 26.0 | 1.1538 |

**Range 1.139–1.232, median 1.158. A real kernel-local prize.** The 18 dot products per body are
mutually independent, so the removed multiplies were on the critical resource, not off it.

### C — colored solve rank body, scalar reference (the shipped default path)

| Run | split median µs | fused median µs | ratio | note |
|---|---|---|---|---|
| G1–G5 | 1 640.0–1 759.0 | 1 783.7–1 855.6 | 0.9082, 0.9479, 0.9429, 0.9222, 0.9476 | the two arms never overlap |
| P1 | 1 646.1 | 2 013.8 | 0.8174 | **contaminated**: fused p90 2 063 vs a 1 810 floor in every other run; reported, not used |
| P2 | 1 655.1 | 1 813.5 | 0.9127 | clean |
| P3 | 1 723.1 | 1 805.6 | 0.9543 | clean |

**Clean range 0.908–0.954. Fusing the scalar reference makes it 5–9 % slower** in the
transcription, by the packing mechanism shown in the opcode census — a mechanism the in-tree
`solve_color` exhibits too (section "In-tree confirmation on the shipped binary"), so the sign is
the real kernel's and the magnitude is the transcription's. This arm is the default, and since
2026-09-03 that rests on `simd_solve` alone: `PhysicsConfig::simd_solve` defaults to `false`
(`resources.rs:449`) and it is the only flag `solve_color_dispatch` (`solver/colored.rs:1711`)
consults when choosing between `solve_color_avx2` and the scalar `solve_color` measured here.

> **This sentence read "`PhysicsConfig::simd` and `PhysicsConfig::simd_solve` both default to
> `false` in `resources.rs`, and no production call site sets either" until 2026-09-03.**
> `simd` now defaults to `true` (`resources.rs:444`). It never selected this arm — it is read into
> a different local (`solver/colored.rs:3083`) and reaches only `simd::apply_gravity` (`:3114`) and
> `simd::refresh_inertia` (`:3151`) — so the measurement above is unaffected, in mechanism and in
> value. The "no production call site" half still holds for `simd_solve`: the sole assignment in
> the workspace is `tests/colored_acceptance_simd_o7.rs:132`. The decision file's section 3 works
> through what the flip does and does not move.

### E — inertia refresh, scalar reference (10 011 bodies per pass)

| Run | split median µs | fused median µs | ratio |
|---|---|---|---|
| P1 | 111.8 | 96.3 | 1.1610 |
| P2 | 111.2 | 96.0 | 1.1583 |
| P3 | 115.0 | 101.2 | 1.1364 |

**Range 1.136–1.161 — an upper bound for the in-tree kernel.** Only three runs (this row was
added by the design pass); tight, and the mechanism is the same as B's *in the transcription*,
whose split arm has no packed ops. The in-tree `refresh_inertia_scalar` **is** partially packed
(10 `vmulps` / 8 `vaddps` / 2 `vsubps`, section "In-tree confirmation on the shipped binary"), and
`mul_add` defeats that packing in the solve; so the in-tree gain from fusing the scalar refresh is
at most this and plausibly much less. Note the scalar refresh is 3.8× slower than the 8-wide one,
so the default path is where the inertia refresh costs most in absolute terms — and it is still
2.2 % of the solve+inertia time.

---

## Step-share model

The colored solver's substep loop (`ColoredSoftStepSolver::step` in `solver/colored.rs`) runs, per
step at the defaults (`substeps = 4`, `relax_iterations = 2`): 4 × `apply_gravity`, 4 ×
`warm_start_apply`, 4 × (1 + 2) = **12 `solve_all_colors` passes**, 4 × `position_integrate`
(scalar by O1's measured choice), **4 × `refresh_inertia`**, then restitution and the warm store.
Using the per-pass medians above and counting only solve + inertia (which flatters the inertia
share — broadphase, narrowphase, graph build, gather and apply are all omitted):

| Path | inertia share of solve+inertia | saving from fusing inertia alone | runs |
|---|---|---|---|
| SIMD (`simd = true`, `simd_solve = true`) | 2.56–2.57 % | **0.341–0.351 %** | P1–P3 |
| Scalar (the default) | 2.18–2.21 % | **0.261–0.307 %** | P1–P3 |

Every in-tree criterion bench of this solver has a run-to-run spread of several percent (the
memory record on `parallel_solve` and VB-P1d says so directly). A 0.3 % change cannot be
distinguished from noise by the instruments that would have to certify it.

---

## Fusion-site counts per kernel

Counted in the compiled loops (the census above) and cross-checked against the source shape:

| Kernel | Fusible sites | Basis |
|---|---|---|
| `cross8` | 3 | each component is `mul; mul; sub` → one `vfmsub` |
| `dot8` | 2 | three-term chain |
| `mat3mulvec8` | 6 | 3 × `dot8` |
| `effective_mass_x8` | 28 | 2 × (cross 3 + mat3mulvec 6 + cross 3 + dot 2) |
| `pointvel_x8` | 3 | cross only (the adds are plain) |
| `apply_impulse_blend_x8` | 12 | 3 (`p*invm + lin`) + cross 3 + mat3mulvec 6 |
| One contact's rank body | ~151 predicted; **145 emitted** | 3 × effective mass + 4 × point velocity + 4 × apply + normal delta + 2 friction deltas + `lensq` + 3 friction impulse |
| `quat_to_mat3_x8` | ~3 | only the three `1 - 2*(..)` diagonals fuse cleanly; left split in both arms here |
| `mat3_mul_x8` | 18 | 9 three-term dots × 2 |
| `refresh_inertia` per body | **36 emitted** | 2 products × 18 |
| `sd_sphere_x8` | 2 of 10 ops | `mul,mul,mul,add,add,sub,sub,sub,sub,sqrt` |
| `sd_box_x8` | 2 of 19 ops | dominated by `max`/`min`/`sub` |
| `smin_x8` | ~1–2 of 11 | polynomial smooth-min with an exact `div` |
| `combine_x8`, `clamp01_x8` | 0 | pure `max`/`min`/`blend` |
| `apply_gravity_avx2` | 3 of 6 | streaming, memory-bound |
| `position_integrate_block_x8` | ~16–20 of 54 | ships scalar by O1's measurement |
| `narrowphase/box_box.rs` | small | 28 `*` in 959 lines, branch-bound |

---

## Limits of these measurements, stated so they are not over-read

- **Synthetic scene.** Bodies are gathered by a strided index pair, not by a real colour's group
  CSR, so the gather's cache behaviour is a model. That cuts toward *over*-stating the FMA prize
  (a costlier real gather dilutes the arithmetic further), and the L1-resident variant bounds it
  from the other side.
- **Not an in-tree criterion run.** The transcriptions are validated bit-exact against each other
  (split-vs-split 0/29 752 and 0/90 072), not against the live kernels, which were under
  concurrent edit. The decision does not rest on a number finer than the spreads shown. Revision 2
  adds the in-tree disassembly, which confirms the *mechanisms* (packing present in the split
  scalar kernels; zero fused ops anywhere) but not the *magnitudes* — the in-tree fused variants
  were not built, because building them means editing `crates/`.
- **Profile.** The scratch timings use `lto = "fat"`, `codegen-units = 1`; the shipped profile is
  `codegen-units = 16`, no LTO (no `[profile.release]` in the workspace). Cross-crate inlining of
  `boyko_math` into the physics loops is therefore *better* in the scratch build than in the
  shipped one, which flatters both arms equally and does not change any sign.
- **Two rows the in-tree census weakens, both in the direction of the decision:** row E's
  1.14–1.16× is an upper bound (the in-tree split scalar refresh is packed; the scratch one was
  not), and the step-share saving derived from it is correspondingly an upper bound.
- **Contention.** Two rows are marked contaminated (G2 on A; P1 on C) by their own `p90`; both
  are reported. The sign of every conclusion is the same with or without them.
- **Runs.** A, B, C, D have eight invocations (seven clean for C); E has three. The D comparison
  is directional and agrees with A.
- **graphify** was not on `PATH` in this environment; navigation used one Grep/Read fallback per
  the hook's rule.
