# Open items

The construct backlog was **fully ratified by the owner on 2026-08-28**; what remains open here is
only the unverified-claims ledger below. Implementation folds into rung R3 (see
[`CAMPAIGN.md`](CAMPAIGN.md)); the specs live in [`CONSTRUCTS.md`](CONSTRUCTS.md).

## Constructs — RATIFIED 2026-08-28

| # | Construct | Ruling |
|---|---|---|
| O1 | `set Combat order (…) when (…);` | **adopt** — Aether-only, no engine impact; completes the `sets` group (set-level `run_if` evaluates once per set instead of once per system) |
| O2 | `system exclusive flush(w: world)` | **adopt** — the accidental verbatim-escape exclusivity becomes a named, arity-checked form; a plain system with a verbatim `&mut EcsMaster` is refused with a pointer at `exclusive` |
| O3 | `gpu` marker on `system` | **adopt** — emits `SystemConfig::gpu()`; the marker is deliberately non-inferable in the kernel, so the language surface is the only ergonomic route |
| O4 | `relation Likes -> LikedBy { … }` | **adopt variant (b)** — one construct mints both sides; the private reverse-index field becomes generator-internal and the cross-references cannot desync |
| O5 | `resource` | **ratified** as specified. The owner floated a broader "resources" campaign — scope to be clarified when it opens; the construct does not wait for it |
| O6 | hierarchical tags `tags { Weapon.Ranged.Rifle; }` | **adopt with both gates** (archetype-count ceiling computed in the expander; `#[require]` only for the `sticky` vocabulary class since requires are never removed). Honest framing recorded: zero runtime difference vs manual tags — this is correctness sugar (the ancestor-implication invariant kills the silent partial-attach class), valuable when queries span taxonomy levels |
| O7 | `attributes` (GAS Base/Mods/Current) | **adopt as the foundation** — the recompute-from-base pattern is the part hand-rolled buff code gets wrong (field-list desync; order-dependent removal drift); the `effect` construct layers on later without rework |
| O8 | event payload binding `on Damage(dmg) if … => …` | **adopt** (the global-machine twin of the per-entity `arg` slot; first event passing the guard wins, consistent with first-declared arbitration). Binding guards (`if let`) follow as a second step on demand |

## Open questions elsewhere in the corpus

- **Gaia** — the data language (scenes, UI documents, the DataAsset/DataTable analog), named by
  the owner 2026-08-28: *Aether is for logic, Gaia is for data*. It absorbs the scene-format
  campaign's pipeline decisions (own text → build-time bake with reflection → binary, zero runtime
  reflection). Research launched; a plan directory of its own follows the research.

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
