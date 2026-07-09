# docs/archive

Completed, historical planning material lives here: per-phase plan / research /
critic-round / results documents (`PHASE-*`, `GUI-P*`), and the design/plan docs
for features that have since shipped on branch `ecs` (relations, observers,
cloning, required components, enable-tag, input, demo, std-lib, the physics
foundation series, the RHI trait plan, the Phase 4/5/6 seam plans, etc.). Each
file carries a `> STATUS: COMPLETED — archived …` header; the authoritative
record for what actually landed is the matching `*-RESULTS.md` plus the git
history.

These are kept for provenance, not as current navigation. For "where is X?" start
at [../FEATURE_MAP.md](../FEATURE_MAP.md); for the subsystem catalog see
[../SYSTEMS.md](../SYSTEMS.md); for cross-crate architecture see
[../ARCHITECTURE.md](../ARCHITECTURE.md). Active / forward-looking plans (parked
render designs, deferred serialization phases, the audit and roadmap docs) stay
at the `docs/` top level.

To search the history of an archived file, `git log --follow` tracks it across
the move, e.g.:

```sh
git log --follow -- docs/archive/PHASE-19-RESULTS.md
git log --oneline --follow -- docs/archive/RELATIONS-API-PLAN.md
```
