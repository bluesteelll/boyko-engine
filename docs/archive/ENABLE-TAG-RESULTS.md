> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# EnableTag — Results (enable-bit, non-fragmenting tag backend)

Branch `ecs`. EnableTag is the **second tag storage backend** alongside the
Phase-22 archetype-signature tags. Presence is encoded in a per-archetype
**paged bitset** (one bit per row); toggling is an O(1) atomic bit
read-modify-write — no archetype migration, no fragmentation, no spawn-time
tick-pool floor. The cost is a per-row bit test when a query names an enable
tag, and change detection (`Added`/`Changed`) is compile-rejected for bitset
tags (the bit carries no per-row tick).

Authoritative design: [`ENABLE-TAG-PLAN.md`](ENABLE-TAG-PLAN.md) +
[`ENABLE-TAG-PLAN-AMENDMENT-D7.md`](ENABLE-TAG-PLAN-AMENDMENT-D7.md)
(bounded data-less global scan in v1). Subsystem catalog:
[`SYSTEMS.md` §3.8](../SYSTEMS.md), [`FEATURE_MAP.md`](../FEATURE_MAP.md),
[`ARCHITECTURE.md` decision 14](../ARCHITECTURE.md).

## Status: COMPLETE

All six implementation waves landed, reviewed, Miri-clean, and the sacred
0%-gate is held. Public + internal docs and a demo dogfood shipped.

| Wave | Commit | Contents |
|------|--------|----------|
| 1 | `7103520` | registry storage-kind, paged enable-bit store, presence oracle |
| D7 | `3a13f0a` | amendment: bounded global-scan `Query<(), Enabled/Disabled<A>>` |
| 2 | `3bbd289` | archetype wiring + O(1) toggle API |
| 3 | `25e36ba` | `Enabled`/`Disabled` filters, migration bit-copy, bounded global-scan |
| 4 | `6d2dbfa` | `Added`/`Changed` reject + deferred toggle + dynamic terms + QueryView get |
| 4-rev | `5220cbc` | comment-precision follow-up (per-row gate is a runtime loop-invariant branch, not a const-fold) |
| 5 | `4757419` | `#[component(storage = "bitset")]` derive arm (+ review hardening) |
| 6 | `14e9fb2` / `5a431f2` / `482fc30` | benches / book + internal docs / demo dogfood |

## Verification

Build/test toolchain: **`stable-x86_64-pc-windows-gnu` (rustc 1.96.0)** — this
matches CI's `dtolnay/rust-toolchain@stable`. (See the toolchain footgun note
below: the machine's *default* msvc toolchain is broken on this box.)

- `cargo check --workspace --all-targets`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: green — lib 662, `enable_tag_derive_step10` 13,
  every enable integration suite + trybuild fixtures green; demo dogfood 2 +
  unit tests green.
- **Miri-TB** (`-Zmiri-tree-borrows -Zmiri-ignore-leaks`): clean — 67 lib
  enable-units + 35 enable integration tests (`migration_step6` 14,
  `step9` 5, `derive_step10` 13, `change_detection_positive` 3), no UB.
- **Code review**: Wave-4 adversarial retro-review APPROVED (0 CRITICAL/MAJOR);
  Wave-5 adversarial 3-dimension review workflow — 0 confirmed CRITICAL/MAJOR
  (the one CRITICAL raised was a false alarm from a reviewer running on the
  broken default msvc toolchain; refuted by the gnu-1.96 oracle).
- **loom**: not applicable to v1 — toggle is a `&mut EcsMaster`
  structural-class op (no concurrent toggler), and the presence bitset /
  `enable_generation` are publish-once/never-retire (no lock-free
  producer-consumer to model beyond `par_iter`, which Miri-TB covers).

## Benchmarks (`benches/enable_tags.rs`, measurement-time, gnu-1.96)

| Bench | Result | Per-unit | Note |
|-------|--------|----------|------|
| `enable_toggle` | 13.33 ns / enable+disable pair | ~6.67 ns / toggle | O(1) bit RMW, no migration |
| `query_iter_enabled` | 66.43 µs / 10k rows | ~6.64 ns / row | filter adds ~3.2 ns/row over the ~3.4 ns/row unfiltered baseline |
| `spawn_with_enable_tag/plain_spawn` | 148.98 µs / 10k | ~14.9 ns / spawn | baseline |
| `spawn_with_enable_tag/spawn_then_enable` | 224.36 µs / 10k | enable pass +~7.5 ns/e | same archetype — no spawn-time churn |
| `enable_toggle_large_archetype` | 13.31 ns / pair | ≈ page-0 toggle | confirms the page allocation is lazy/bounded; toggle is O(1) regardless of page index |

Aspirational plan targets (toggle `<5 ns`, `query_iter_enabled ≤1.5 ns/row`)
are not met; the measured costs are realistic for an uncontended atomic RMW plus
the paged double-indirection. The headline properties — O(1) non-fragmenting
toggle, no spawn floor, lazy bounded pages — all hold. The gap is an
optimization opportunity, not a correctness issue (see follow-ups).

## The sacred 0%-gate: HELD

A query that does not name an enable tag must be byte-identical pre- vs
post-EnableTag. This is guaranteed architecturally (the derive emits empty
token streams for non-bitset components; the per-row enable test sits behind a
single loop-invariant `EnableTerms::is_empty()` guard that the compiler hoists
to one predicted-not-taken branch) and was confirmed empirically.

**Methodology (drift-resistant).** A first criterion `--save-baseline` /
`--baseline` A/B across a checkout was contaminated by clock/frequency drift
during the recompile gap — it reported physically impossible bidirectional
±13% swings across sub-benchmarks of the same file (a const-gated no-op cannot
make one sub-bench +13% and another −13%). A calibration (two consecutive
HEAD runs, ~0.4% / "No change") proved the box was actually stable. The clean
gate was then taken with both bench binaries pre-built (pre-EnableTag baseline
`0429d97` in a throwaway git worktree, HEAD on `ecs`) and measured
**alternately in the same quiet window**, which cancels slow drift.

**Result** — `query_state_iter` (drives the EnableTag-touched query hot path),
BASE = `0429d97` (pre-EnableTag) vs HEAD = `ecs`:

| sub-bench | BASE (ns/row) | HEAD (ns/row) |
|-----------|---------------|---------------|
| /1000 | 3.402 – 3.405 | 3.403 – 3.405 |
| /10000 | 3.402 – 3.430 | 3.405 – 3.407 |
| /100000 | 3.398 – 3.402 | 3.417 – 3.424 |

All 12 measurements lie in 3.398–3.430 ns/row; BASE and HEAD are
indistinguishable (max spread +0.94%, below the ±2% gate threshold and within
the box's own ~0.4% run-to-run noise). **0%-gate held.**

Lesson (recorded): on this box, cross-commit criterion A/B via
`--save-baseline` + recompile + `--baseline` is drift-contaminated. The
reliable method is pre-build both binaries (worktree for the baseline) and
measure alternately in one window; always calibrate with a same-binary
back-to-back run first.

## Deferred follow-ups (filed, NOT done here)

- **Beat-Bevy perf-gap closure** — design APPROVED
  ([`PERF-GAP-BEAT-BEVY-PLAN.md`](../PERF-GAP-BEAT-BEVY-PLAN.md)); sequenced after
  EnableTag.
- **Positive-term archetype cull** — `cull_enable_archetypes` is a deliberate
  NO-OP today; a positive-term `Query<&D, Enabled<A>>` filters per-row instead
  of culling whole non-present-A archetypes. Needs a `QueryFilter`
  polarity/cull leaf; acceptance bench `query_iter_enabled_culled`. This is
  also the path to closing the `query_iter_enabled` per-row gap above.
- **`enable_toggle` / `query_iter_enabled` micro-opt** — cache the page pointer
  per 4096-row block to remove one dependent load from the per-row bit test.
- **BUG-ENABLE-PRE-1** — `Or<(Changed<A>, With<B>)>` row-leak in a B-lacking
  archetype. Pre-existing (predates EnableTag); filed as an isolated bugfix
  wave (M3). Not touched here.
- **BUG-ENABLE-PRE-2** — `QueryView::get`/`get_mut` silently ignore
  `Changed<C>`/`Added<C>`. Pre-existing; EnableTag added only a rustdoc note
  (C3-r7-c) + a compile-reject of the `Enabled`+`Changed` mix, so the
  misleading partial-filter shape cannot be constructed. The retrofit of change
  detection into point lookups is an isolated wave. Not touched here.

## Toolchain footgun (dev-env note, not a code issue)

The machine's **default** toolchain `stable-x86_64-pc-windows-msvc` (rustc
1.92.0) cannot link on this box — a non-MSVC `link.exe` shadows the MSVC linker
on `PATH` (`link: extra operand … Try 'link --help'`), so every msvc build
fails at the build-script link stage. Build/test/bench with the gnu toolchain:
`rustup run stable-x86_64-pc-windows-gnu cargo …`. CI is unaffected (it runs
`@stable` on Linux). Trybuild `.stderr` fixtures are pinned to current stable
(1.96) and are green on the gnu toolchain and CI; older local toolchains render
the const-eval-panic `panic.rs` frame differently (inherent trybuild
version-fragility, shared by the pre-existing fixtures).
