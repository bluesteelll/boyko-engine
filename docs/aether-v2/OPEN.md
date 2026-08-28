# Open items — not yet ratified by the owner

Everything in the campaign that still needs an owner decision, plus the claims that remain
unverified. Each construct row carries a one-line recommendation so the whole section can be closed
in one review pass, the way the machine forks were.

## Constructs proposed but never discussed

| # | Construct | What it closes | Recommendation |
|---|---|---|---|
| O1 | `set Combat order (…) when (…);` | `configure_set` (set-level ordering/conditions) is unreachable from the language; `sets (Combat)` can only reference | adopt — it is the missing half of the `sets` group |
| O2 | `system exclusive flush(w: world)` | exclusive systems (`fn(&mut EcsMaster)`) compile through the verbatim escape BY ACCIDENT — no arity check, no mixing refusal | adopt — one keyword, two refusals |
| O3 | `gpu` marker on `system` | `SystemConfig::gpu()` (GPU-compute, dispatcher-solo at the apply window — the only sound site for `!Send` RHI recording) has no language surface | adopt for an engine with an in-house RHI |
| O4 | `relation Likes -> LikedBy { linked_despawn, allow_self }` | the ONE unadopted component-design decision: the reverse side requires a PRIVATE collection field, colliding with the "everything pub" rule | adopt the synthesizing construct (one declaration mints both sides; privacy becomes the generator's internal business) over a visibility exception |
| O5 | `resource` ratification | used by the accepted machine design; formally never approved as a construct | ratify as specified in CONSTRUCTS.md |
| O6 | hierarchical tags `tags { Weapon.Ranged.Rifle; }` | UE GameplayTags ergonomics via a `#[require]` lattice; `With<Weapon>` catches all descendants for free | adopt with the two hard gates: an archetype-count ceiling computed in the expander, and a split vocabulary (only a `sticky` class gets `#[require]`, because requires are never removed) |
| O7 | `attributes` (GAS Base/Mods/Current) | one declaration → three POD structs + one recompute system under `Changed<Mods>`; makes the three field lists unable to desync; order-independent buff removal | the judge escalated it as a VALUES call: without an `effect` construct nobody can write a buff through it — decide whether it ships alone or waits for effects |
| O8 | event payload binding `on Damage(dmg) if dmg > 5 => …` + binding guards (`if let Some(x) = …`) | an event's payload is not bindable in a transition at all today | adopt — small, closes a real hole; the per-entity `arg` slot already covers the machine side, this is the global-machine twin |

## Open questions elsewhere in the corpus

- The participant-context dead datum: **RESOLVED** (F6 — a router debug_assert; recorded in
  OPEN-QUESTIONS with the date).
- The both-chains-run machine defect: recorded in OPEN-QUESTIONS; fix scheduled at R2 (route
  merge); needs its red test BEFORE the fix.
- `#[require]` of a bitset tag: believed to panic at first expansion (`pool_id_for(...).expect`)
  — **still a code-reading claim**; write the red test first (E11).

## Unverified numbers carried by the plan

| Claim | Status |
|---|---|
| the jump-table probe's figures (3.4–4.0 ns/row, invariance across working sets) | measured once, single machine, worktree probe; promote the probe into `crates/boyko_ecs/tests/` under `#[ignore = "slow: …"]` on owner's word |
| "37 of 74 `Query<…>` declarations carry a chunk-refused term" | the counting regex was flagged as truncating multi-line declarations — recount before citing in a spec |
| spatial/event cost figures (µs-level) | models, not measurements; criterion is the oracle |
| `iter` vs `for_each_chunk` A/B on the machine-pass shape | does not exist in the repo at all; listed at E12 |
| loom/Miri coverage for the `&self` lane path | promised by the design, not yet written |
