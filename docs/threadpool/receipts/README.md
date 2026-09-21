# `tb-neg-m2w-<seed>.stderr` — the KE16 M2w negative control's receipts

One file per seed (`0`, `1`, `7`, `15`), written by `scripts/tb_neg_gate.ps1` (or `.sh`) and
censused by `crates/boyko_threadpool/tests/tb_neg_m2w_arm_present.rs`. Each is the stderr of one
`cargo miri test -p boyko-threadpool --features tb-neg-m2w --test tb_neg_m2w_block_reference` run
under `-Zmiri-seed=<seed>`, and its value is ONE attributable red: `deallocation through <tag> …
is forbidden`, with the protected tag born at `scoped.rs:<the line of `_cell: &ScopedCell<`>`, the
accessed tag born at `block.rs:<the line of `unsafe { alloc(layout) }`>`, and `ScopeBlock::free_all`
as the deallocating frame. Both line numbers are derived from the source by the gate on every run.

**Checker: `nightly-x86_64-pc-windows-msvc`, since 2026-09-21** (rung AH of the unification plan).
The four `tb-neg-m2w-<seed>.stderr` were produced by that nightly's miri
(`miri 0.1.0 (a36d05efab 2026-09-09)`) from tree `54e186d9` plus this change's own doc-comment
edits, in `D:/wt/_targets/ah-miri-msvc`.

**`tb-neg-m2w-<seed>.gnu.stderr` is the same run on `nightly-x86_64-pc-windows-gnu`**
(`miri 0.1.0 (8925ea358a 2026-08-20)`, `D:/wt/_targets/ah-miri-gnu`), from the SAME tree, on the
same day. It is kept beside the msvc receipt so that the checker move is attributable: the gnu
files are the last receipts the previous checker produced, and they are not what the census reads.

## What the side-by-side showed (2026-09-21, 4 seeds × 2 nightlies)

Identical on every seed and both nightlies: the diagnostic kind, the accessed-tag site
(`block.rs:482:28`), the protected-tag site (`scoped.rs:303:17`), the deallocating frame
(`ScopeBlock::free_all` at `block.rs:341:17: 344:18`), the thread name, and cargo's own
`(exit code: 1)`. Both gates reported `red on 4/4 seeds`, each seed with `running 1 test`, and
`--list` printed `1 test, 0 benchmarks` on each nightly before the run.

What differs between a `.stderr` and its `.gnu.stderr`, all of it instrument identity and none of
it the property:

- the toolchain path in the two `core` backtrace frames (`…\toolchains\nightly-x86_64-pc-windows-msvc\…`
  vs `…-gnu\…`; the `core` line numbers are the same, `ptr/mod.rs:848` and `mem/mod.rs:1049`);
- the target-dir path in the `Running` and `process didn't exit successfully` lines;
- Miri's tag and allocation ids (`<176340>`, `alloc60834`, …), which are per-run;
- a cargo preamble of `non_kebab_case_bins` manifest warnings on the msvc receipts only — the
  2026-09-09 cargo lints workspace bin names, the 2026-08-20 one did not;
- `Finished … in N.NNs`.

What differs between these receipts and the ones they replace (committed by `d51b4ced`,
2026-09-08, gnu nightly, tree `D:\wt\threadpool`), and is the TREE rather than the host — the gnu
run from this tree shows every one of them too:

- frame 4 `Scope::join_after_body` (`scope.rs:1230`) between `drop` and `PoolInner::install`,
  and `thread_pool.rs:298` / `:492` / `scope.rs:1470` in place of `:341` / `:534` / `:1479` —
  introduced by `1693234d` (A6 panic propagation, 2026-09-17) and the seven other
  `boyko_threadpool/src` commits since `d51b4ced`;
- the offset inside the chunk on the first line, `[0x0]` or `[0x38]`: which bump slot the
  protected cell occupies when `free_all` runs. Seed-dependent on both nightlies from this tree
  (gnu `0x38 / 0x0 / 0x38 / 0x38`, msvc `0x38 / 0x38 / 0x0 / 0x0` for seeds `0 / 1 / 7 / 15`);
  `d51b4ced`'s four read `[0x0]`.

## Re-running

`pwsh`/`powershell -NoProfile -File scripts/tb_neg_gate.ps1` overwrites the four
`tb-neg-m2w-<seed>.stderr`. For a gnu comparison run, `TB_NEG_TOOLCHAIN=+nightly-x86_64-pc-windows-gnu
bash scripts/tb_neg_gate.sh` writes to the same four names — move them to `.gnu.stderr` before the
msvc run, or the msvc receipts are lost. Use a target dir of its own for each nightly, and delete
its `build/boyko-threadpool/` before a re-run after a source edit: `cargo-miri` writes an empty
dep-info and reports `Fresh` on the next build, so require a `Compiling boyko-threadpool` line.
