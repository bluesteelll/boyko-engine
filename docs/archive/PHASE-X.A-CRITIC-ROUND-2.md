> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase X.A — Architecture Critic, Round 2

**Verdict:** APPROVED WITH FOLLOW-UP — the developer can begin Wave 1
(Steps 1A/1B/1C, all parallelizable) immediately. None of them touch
§2.4 closure docs, §8.1 toolchain plumbing, or §12 Step 8A. W2.1 and
W2.2 land entirely on Wave 8 surfaces (bench frequency docs,
toolchain file path), which the dependency graph defers to last. The
architect should land a Round 3 patch on W2.1 + W2.2 (plus optionally
N2.1 / N2.2 / N2.3) before Wave 8 starts, but Waves 1-7 do not block
on it.

---

## Section-by-section verification of Round 1 patches

- **W1** (§10.7 inline table collapse): resolved (✓). Two
  contradictory rows merged into one row stating `#[inline]` on the
  public `Query::for_each_chunk` / `Query::par_for_each_chunk` and
  **no annotation** on the internal `for_each_chunk_impl` /
  `par_for_each_chunk_impl` drivers. Mirrors `par_iter.rs:166-167,
  216-217, 244` precedent verbatim.
- **W2** (§2.4 + §1.2 closure-frequency callout): partially
  resolved (~). The "once per archetype-subrange, not once per
  archetype" callout is correct in shape, but the **arithmetic in
  both §2.4 and §1.2 is wrong** — see W2.1 below.
- **W3** (§8.1 toolchain pin scope): partially resolved (~). §8.1
  itself correctly describes the per-package mechanism, but §12 Step
  8A line 1239, the §12-trailing "New files" list line 1394, and
  §14 Q4 line 1354 **still reference the workspace-root
  `rust-toolchain.toml`** — see W2.2 below.
- **W4** (§1.2 "Allocations per frame" row): resolved (✓). Row now
  accurately states "0 in steady state; one
  `Box<UnsafeCell<QueryDataState<D, F>>>` per **new** `(D, F)` pair
  on first use" with the `OnceLock<...>` reference correctly anchored
  to `query_state_cache`.
- **W5** (Step 1A test ordering): resolved (✓). The
  `buffer_ptr_is_simd_aligned` test is now first with explicit
  "Test FIRST" prefix and gating rationale.
- **N1** (§11.6 asm size check): resolved (✓) with a methodology
  caveat (N2.3 below — `wc -c` on textual asm doesn't measure encoded
  instruction bytes; that's noise the developer will need to interpret).
- **N2** (§5.2 `Or<F>` SAFETY comment): resolved (✓). The comment
  now explicitly explains why `F: ArchetypalQueryFilter` suffices via
  the inner-tuple monomorphisation argument.
- **N3** (§6.2 cost arithmetic sourcing): resolved (✓). Cited
  `component_registry.rs:47` (MAX_COMPONENTS = 512, verified) and
  `archetype_bit_set.rs:7` (MAX_ARCHETYPES = 1024, verified).
- **N4** (§11.2 trybuild aliasing-test path): resolved (✓). Wording
  now explicitly says "test must declare a system fn taking
  `Query<(&mut T, &mut T), ()>` and register it in a `Schedule`" with
  the `query_cold_init` → `init_state` (not `init_access`)
  verification anchor.

## Sanity check on previously approved decisions

Sibling trait (§4), marker filter (§3), alignment lift (§6), NCD
elision via type bounds (§7), and parallel granularity (§9) all read
unchanged. No regression.

---

## New issues from the patched wording

### W2.1 (Important) — §2.4 and §1.2 closure-frequency arithmetic is off by ~10×

**Where:** §2.4 line 255 ("A 100k-row archetype with default
`BatchingStrategy` yields ~100 invocations of `Func` (one per
~1024-row sub-range)") and §1.2 line 40 ("default `BatchingStrategy`
derives `batch_size` from `MIN_ARCHETYPE_FOR_PARALLEL`").

**Problem:** `BatchingStrategy::chunk_size`
(`par_iter.rs:117-123`) computes `batch_size = (entity_count /
(worker_count * batches_per_thread)).clamp(MIN_ARCHETYPE_FOR_PARALLEL,
usize::MAX)`. `MIN_ARCHETYPE_FOR_PARALLEL = 1024` is a **floor**
(lower bound on the clamp), not the derivation source. For a 100k-row
archetype on an 8-worker pool with default `batches_per_thread = 1`:

- `raw = 100000 / 8 = 12500`
- `batch_size = clamp(12500, 1024, usize::MAX) = 12500`
- closure invocations = `100000 / 12500 = 8`, not ~100.

The pattern is closure invocations ≈ `worker_count ×
batches_per_thread` for medium-large archetypes; it only reduces to
`entity_count / 1024` when `entity_count / worker_count < 1024` (i.e.,
small archetypes whose `raw` falls below the floor). This is
fundamentally a different mental model from what §2.4 / §1.2 claim.

**Why important:** the docs become the canonical reference for
user-facing accumulator sizing. A user reading the rustdoc and sizing
a sharded `[AtomicF32; N]` accumulator will pick the wrong N (100
instead of ~num_threads), wasting cache lines and slowing combine.
Phase 9 already has `worker_count`-shaped TLS storage primitives —
locking in the wrong shape contradicts the principle of staying
consistent with established patterns.

**What is needed:** rewrite §2.4 and §1.2 frequency wording to
reflect the real formula. Suggest either:

(a) "≈ `min(worker_count × batches_per_thread, entity_count /
    MIN_ARCHETYPE_FOR_PARALLEL)` invocations per archetype" if a
    single-formula statement is preferred, or
(b) two regime examples: a small-archetype case where the floor
    binds, and a large-archetype case where the per-worker shape
    dominates.

Also: drop the §9.1 line 770 "1024-row chunk × 1 ns/row = 1 µs of
work" example since it mirrors the same arithmetic confusion.

### W2.2 (Important) — W3 patch left three stale workspace-root mentions

**Where:** §12 Step 8A line 1239, the §12-trailing "New files" list
line 1394, §14 Q4 line 1354.

**Problem:** §8.1 (the W3 patch zone) correctly switches to
per-package `crates/bench_bevy_vs_boyko/rust-toolchain.toml`. But
three downstream references still say workspace-root
`D:\claude\BoykoEngine\rust-toolchain.toml`:

- Step 8A line 1239: "*File (new)*:
  `D:\claude\BoykoEngine\rust-toolchain.toml` per §8.1." —
  contradicts §8.1.
- New-files list line 1394:
  "D:\claude\BoykoEngine\rust-toolchain.toml (Wave 8A)" — same.
- §14 Q4 line 1354: "Resolved in the prompt + §8 → nightly with
  `f32::algebraic_add` + `rust-toolchain.toml` at workspace root." —
  same.

**Why important:** the developer's checklist for Step 8A will follow
the §12 instruction literally (it's the actionable line); they'll
create the file at the workspace root, which is exactly the
CI-breaking outcome W3 was meant to prevent. CI verification:
`.github\workflows\ci.yml:20-22` uses `dtolnay/rust-toolchain@stable`
action which **overrides** the `rust-toolchain.toml` channel
selection — so CI is safe on the action side. But `cargo bench
--no-run` in the `bench-compile` job (line 56-63) doesn't use
`+nightly`; if Step 8B adds `#![feature(float_algebraic)]` to
`g6_for_each_chunk.rs` and Step 8A creates a workspace-root pin,
the bench-compile job will try to compile the bench on the
action-installed stable toolchain (rust-toolchain.toml is ignored
when the action pins channel) — meaning the `#![feature]` will fail
at the language-feature gate, not the toolchain gate. This is the
actual breakage path W3 anticipated; the patch fixed it in §8.1 but
didn't propagate.

**What is needed:** update Step 8A, the new-files list, and §14 Q4
to match §8.1's per-package decision. Specifically Step 8A should
write `crates/bench_bevy_vs_boyko/rust-toolchain.toml`, and the
file-list should reflect the same path.

### N2.1 (Nitpick) — §10.7 inline table omits `QueryView::for_each_chunk` / `QueryView::par_for_each_chunk`

**Where:** §10.7 table line 892.

The W1-patched row covers "`Query::for_each_chunk` /
`Query::par_for_each_chunk` (public SystemParam methods)" but §2.5 /
Wave 5 / Wave 6 also introduce the `QueryView` mirror methods.
They're equally cross-crate (called via `EcsMaster::query` direct API).
Adding them to the same row ("plus their `QueryView::*` mirrors")
closes the gap without adding rows.

### N2.2 (Nitpick) — Step 1A test name inconsistency with §13 Risk 4

**Where:** Step 1A line 1029 names the test
`buffer_ptr_is_simd_aligned`; §13 Risk 4 line 1323 names it
`simd_buffer_align_lift_holds`. Two names for the same gating test.
Pick one in both places.

### N2.3 (Nitpick) — N1 asm-size methodology measures characters not bytes

**Where:** §1.2 line 43 and §11.6 line 1007.

`cargo asm ... | wc -c` counts characters in the textual disassembly
output (mnemonics, operands, whitespace, comments) — not the encoded
x86-64 instruction byte length the L1i actually carries. The 256 B
target was originally framed as an I-cache budget; matching it
against `wc -c` on textual asm doesn't actually verify the I-cache
claim. A real budget check would use `cargo asm --bytes ...` or
`objdump -d --no-show-raw-insn | wc -l` × average-insn-length, or
simply `objdump -d --disassemble=<symbol> | awk '/[0-9a-f]+:/ {n++}
END {print n}'` for instruction count. Not a blocker — just a
heads-up the developer will produce a number that doesn't match the
stated unit.

---

## Files relevant to this review

- `D:\claude\BoykoEngine\docs\PHASE-X.A-PLAN.md` (Round 2 plan,
  full read)
- `D:\claude\BoykoEngine\docs\PHASE-X.A-CRITIC-ROUND-1.md`
  (Round 1 critique, reference)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\par_iter.rs`
  (verified `BatchingStrategy::chunk_size` formula at lines 117-123
  — W2.1 anchor)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs`
  (verified `query_cold_init` Box allocation at line 1955; `OnceLock`
  wrapping at line 251 — W4 anchor)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\query\state.rs`
  (verified `QueryDataState::new` calls `init_state` not `init_access`
  at line 70 — N4 anchor)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_registry.rs`
  (verified `MAX_COMPONENTS = 512` at line 47 — N3 anchor)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\iters\archetype_bit_set.rs`
  (verified `MAX_ARCHETYPES = 1024` at line 7 — N3 anchor)
- `D:\claude\BoykoEngine\.github\workflows\ci.yml` (verified
  `dtolnay/rust-toolchain@stable` action + `bench-compile` job that
  surfaces W2.2 risk at line 56-63)
- `D:\claude\BoykoEngine\crates\bench_bevy_vs_boyko\Cargo.toml`
  (verified no current `#![feature]` use; new file would be first
  nightly-only bench)

Sources:
- [Overrides — The rustup book](https://rust-lang.github.io/rustup/overrides.html)
- [Workspaces — The Cargo Book](https://doc.rust-lang.org/cargo/reference/workspaces.html)
