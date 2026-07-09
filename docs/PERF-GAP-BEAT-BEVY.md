# Roadmap — Beat Bevy everywhere (perf-gap closure)

**Directive (2026-06-15):** after the entity-bitset tag backend lands, optimize
**every** measured performance gap vs Bevy so boyko-engine is **faster than Bevy
on every benchmark**, not only on the parallel/scheduler workloads it already
wins. Sequenced **after** the entity-bitset feature (do not interleave — it
touches the spawn/query hot paths the entity-bitset work also touches).

## Measured baseline vs Bevy 0.18.1

Same-machine A/B (each boyko/Bevy pair measured back-to-back in one criterion
process, so the ratio is drift-robust). Source: `crates/bench_bevy_vs_boyko`
benches `comparison` (g1–g4) and `comparison_v2` (g2b, g5). Measured 2026-06-15.

### Already ahead — keep ahead (regression guard, not the work)

| Bench | boyko | Bevy | Ratio |
|---|---|---|---|
| Scheduler — 50 empty systems (g1) | 15.1 µs | 24.4 µs | **1.61× faster** |
| par_iter 10k (g3) | 31.9 µs | 109.2 µs | **3.42× faster** |
| single spawn — Commands ×10k (g4) | 351 µs | 427 µs | **1.21× faster** |
| query iter — direct API (g2b) | 6.73 µs | 6.69 µs | ~parity (1.01×) |

### Gaps to CLOSE (this roadmap's targets — must end up < Bevy)

| Gap | boyko | Bevy | Now | Target |
|---|---|---|---|---|
| **spawn_batch 10k** (g5) | 222 µs | 177 µs | **1.25× slower** | **< Bevy** |
| **query iter via SystemParam** (g2) | 7.86 µs | 6.70 µs | **1.17× slower** | **< Bevy** |
| Phase-22.1 spawn tag floor (2d+2t) | — | — | +26% over 2d (feature-inherent tick-pool cost) | reduce / make enable-bit path floor-free |

Note: the direct-API query iter (g2b) is already at parity, but the SystemParam
path (g2) is ~17% slower — the gap is in the SystemParam wrapping / dispatch, not
the inner loop. The spawn-batch gap is the headline: Bevy's batch spawn is highly
tuned; boyko's must be profiled (entity-id reservation, pool grow/commit, column
write, tick fill, archetype bookkeeping) and beaten.

## Approach (each gap is its own mini-phase)

Run the full `researcher → architect → architecture-critic → developer →
code-reviewer → tester` pipeline per gap. For each:
1. Profile to attribute the gap to a concrete cost (do not guess — the Phase-22.1
   tick-stamping episode proved the obvious culprit can be wrong).
2. Design the fix with the 0%-gate discipline (the paths already ahead must not
   regress — verify with the same `comparison`/`comparison_v2` A/B).
3. Prove the win with a same-machine A/B vs Bevy (within-run ratio, drift-robust)
   AND a vs-pre-fix A/B (drift-cancelled). Machine must be idle (no games — the
   Dota-2 episode invalidated a full perf pass).

## Out of scope here

- The entity-bitset tag backend itself (its own phase, must land first).
- Reflection / serialization gaps (different roadmap; Bevy has features boyko
  lacks — perf comparison is only meaningful on features both have).

## Status

Filed; **blocked on the entity-bitset feature** (tasks: entity-bitset
design → implement → verify, then this).
