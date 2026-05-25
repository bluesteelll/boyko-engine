# boyko-engine — phased implementation roadmap

This directory is the **single source of truth for sequencing**. Every
phase has one file with: goal, scope, status, sub-phases, dependencies,
exit criteria, and a per-step plan where applicable.

The companion documents are:

- [`docs/AUDIT-2026-05-23.md`](../AUDIT-2026-05-23.md) — historical
  findings catalogue. Audit IDs (`C-xxx`, `M-xxx`, `Q-xxx`) are
  referenced from phase files but never duplicated.
- [`docs/ROADMAP-PHASE-2-PLUS.md`](../ROADMAP-PHASE-2-PLUS.md) —
  legacy combined roadmap. Superseded by this directory. Kept for
  link continuity until all in-flight commits stop referencing it.
- [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md),
  [`docs/SYSTEMS.md`](../SYSTEMS.md),
  [`docs/FEATURE_MAP.md`](../FEATURE_MAP.md) — current-state
  reference. These describe **what is**; this directory describes
  **what will be / why / in what order**.

## How to read this

1. Open this README to see the table of phases and their status.
2. Click into the phase file for the current focus — it has the
   actionable plan.
3. Each phase file links to its audit findings, prerequisite phases,
   and downstream consumers.
4. When starting a fresh session, the agent should be told:
   *"Read `docs/plans/PHASE-NN-<topic>.md`. We are working on it."*

## Status legend

| Marker | Meaning |
|--------|---------|
| ✅ DONE | All sub-tasks landed; verified by tests / benches |
| 🟢 IN PROGRESS | Active work; plan locked; implementation underway |
| 🟡 PLANNED | Architectural plan approved; implementation not started |
| ⚪ DRAFT | Outline only; needs architect cycle before implementation |
| 🔒 BLOCKED | Waiting on another phase / external decision |
| ❌ CANCELLED | Decided against; rationale recorded in phase file |

## Phase index

| Phase | Title | Status | Plan file |
|-------|-------|--------|-----------|
| 1 | Foundations — UB / leak / race fixes | ✅ DONE | [PHASE-01-foundations.md](PHASE-01-foundations.md) |
| 2 | Hot-path performance | ✅ DONE (a-e) | [PHASE-02-hot-path-performance.md](PHASE-02-hot-path-performance.md) |
| 3 | Memory-safety hardening | ✅ DONE (a-c), 🔒 BLOCKED (d) | [PHASE-03-memory-safety.md](PHASE-03-memory-safety.md) |
| 4 | Architecture refactors | ✅ DONE (a-b), 🔒 BLOCKED (c) | [PHASE-04-architecture-refactors.md](PHASE-04-architecture-refactors.md) |
| 5 | Cleanup, style, micro-perf | ✅ DONE (a-c), 🟡 OPEN (d) | [PHASE-05-cleanup.md](PHASE-05-cleanup.md) |
| 6 | Event dispatch + double buffer | ✅ DONE | [PHASE-06-event-dispatch.md](PHASE-06-event-dispatch.md) |
| 7 | Fast random component access | 🟡 PLANNED | [PHASE-07-fast-random-access.md](PHASE-07-fast-random-access.md) |
| 8 | System API (typed SystemParam, query DSL) | ⚪ DRAFT | [PHASE-08-system-api.md](PHASE-08-system-api.md) |
| 9 | Parallel scheduler | ⚪ DRAFT | [PHASE-09-scheduler.md](PHASE-09-scheduler.md) |
| 10 | Future work / backlog | ⚪ DRAFT | [PHASE-10-future.md](PHASE-10-future.md) |

## Dependency graph

```
                    Phase 1 (foundations)
                          │
                          ▼
                  ┌──────────────────┐
                  ▼                  ▼
            Phase 2 (perf)     Phase 3 (safety)
                  │                  │
                  └────────┬─────────┘
                           ▼
                    Phase 4 (refactor)
                           │
                           ▼
                    Phase 5 (cleanup)
                           │
                           ▼
                    Phase 6 (events)
                           │
                           ▼
                    Phase 7 (random access)     ◀── current focus
                           │
                           ▼
                    Phase 8 (system API)
                           │
                           ▼
                    Phase 9 (scheduler)
                           │
                           ▼
                    Phase 10 (backlog)
```

Phases 2 and 3 ran partially in parallel (different files). Phases 4
and 5 are largely sequential because they touch the same call sites.
Phases 6 onward depend on the entire prior chain since they add
**new** subsystems on top.

## Principles enforced across all phases

These are non-negotiable constraints — listed once here so each phase
file doesn't repeat them. Every plan defaults to compliance:

1. **Zero runtime overhead** — no `dyn Trait`, `Box`, `HashMap`,
   `Vec::new()` in hot path without explicit justification.
2. **Data-oriented layout** — SoA, hot/cold split, cache-line
   alignment where measurable.
3. **D-cache + I-cache** — both equally weighted. Hot loops fit
   working set in L1d; hot path code fits in L1i.
4. **Lock-free in hot path** — no `Mutex`/`RwLock`/`RefCell`.
5. **Preallocate** — no allocations during the per-frame tick.
6. **SIMD-friendly** — layout amenable to vectorisation.
7. **Measured inlining** — `#[inline]` for cross-crate trivials,
   `#[inline(always)]` only with profiler / asm evidence,
   `#[cold]` for error paths.
8. **`unsafe` justified** — every unsafe block carries a
   `// SAFETY:` block enumerating invariants.

## Git policy

- Author: `Celtokisa <bluesteelll@hotmail.com>` only.
- **No `Co-Authored-By: Claude` tags** in commit messages.
- Branch: `ecs` for all phase work until merge to `master`.
- One commit per logical sub-step. Each commit compiles cleanly and
  passes `cargo test --all-targets`.
- Never `--force` / `--no-verify` without explicit user permission.

## Language policy

All repository artifacts (docs, code, comments, commit messages,
plan files, agent system prompts) are in English. Chat with the user
is in Russian.

Exceptions (intentionally deferred):

- `docs/AUDIT-2026-05-23.md` — historical snapshot, kept verbatim.
- `book/src/*` — public mdBook content; English translation deferred
  until the public API stabilises (after Phase 7).

## Update protocol

When a sub-phase lands:

1. Update its `Status` in the phase file (move from 🟡 / 🟢 to ✅).
2. Add the commit hash(es) and short summary of measured impact.
3. Update this README's table if a top-level phase status changes.
4. Update `docs/FEATURE_MAP.md` if the change introduces or removes
   a feature visible from outside the module.
5. Cross-link to the audit ID(s) being closed.

When a new phase is proposed:

1. Add a `PHASE-NN-topic.md` file in this directory using the
   template at the end of `PHASE-08-system-api.md`.
2. Add a row to this README's table.
3. Add edges to the dependency graph.
4. Run the architect → critic cycle on the plan **before** the
   first developer dispatch.
