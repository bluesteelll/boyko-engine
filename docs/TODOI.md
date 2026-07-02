# List of improvements to do

> **HISTORICAL (pre-Phase-7, `master`-era).** Every item below has since been
> superseded by a landed phase; kept only as a record. Current planning lives
> in [PHASE-13-ROADMAP.md](archive/PHASE-13-ROADMAP.md) (all phases DONE) and the
> per-phase PLAN/RESULTS docs.

## Memory
- Error handling — ✅ `EcsError` + `anyhow` boundaries (Phase 6+)
- Change checks to `debug_assert` — ✅ hot-path discipline (CLAUDE.md rules)
- Expanding containers — ✅ chunked `ComponentPool` + `Arena` lazy commit (X.C)
- Reduce number of indirections during access (probably `Entity -> *Unit` table)
  — ✅ Phase 7 fast random access (~3 ns/lookup); `Unit` itself later DELETED (X.B)
- Iterators — ✅ `Query` iter / `par_iter` / `for_each_chunk` (Phases 8b, 9, X.A)
- Type mismatch handling — ✅ typed `Query<D, F>` DSL (Phase 8b)

## Utils
- `NonMax` integer implementation — ❌ not needed (`EntityId = usize` + NULL slot
  sentinel via `is_null`; revisit only if slot compression ever matters)

## ECS/core
- Archetype iterators — ✅ archetype-walking queries (Phases 8b/12.5 direct API)
