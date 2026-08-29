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

- **The Aether session ballot list is not carried in the repo.** The `F#` ids that appear inside
  Aether rulings (`[delegated F6]`, "F5 dissolves into F4", "(F6: router debug_assert)") come from
  the 2026-08-27/28 session record and are defined **nowhere in this corpus**. The `F#` series that
  IS in the repo — [`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) — is Gaia's, and its numbers do
  not correspond. Aether citations are therefore **substituted** with the ruling that carries the
  substance (`F6` → DECISIONS `M4`), never renumbered into the Gaia series.
- The participant-context dead datum: **RESOLVED for machine `inbox` events only** (DECISIONS M4 —
  a router debug_assert; recorded in OPEN-QUESTIONS with the date). The datum stays **unread for
  every other event**; reading 2 is unfunded.
- The both-chains-run machine defect: recorded in OPEN-QUESTIONS; fix scheduled at R2 (route
  merge); needs its red test BEFORE the fix.
- `#[require]` of an id whose storage kind owns no per-archetype pool — **both poolless kinds,
  Bitset AND Dense**, not the bitset tag alone: believed to panic at first expansion
  (`pool_id_for(...).expect`) — **still a code-reading claim**; write the red tests first,
  parameterised over both kinds (KE11). Disposition is ballot AB-6.

## Unverified numbers carried by the plan

| Claim | Status |
|---|---|
| the jump-table probe's figures (3.4–4.0 ns/row, invariance across working sets) | measured once, single machine, worktree probe; promote the probe into `crates/boyko_ecs/tests/` under `#[ignore = "slow: …"]` on owner's word |
| "37 of 74 `Query<…>` declarations carry a chunk-refused term" | the counting regex was flagged as truncating multi-line declarations — recount before citing in a spec |
| spatial/event cost figures (µs-level) | models, not measurements; criterion is the oracle |
| `iter` vs `for_each_chunk` A/B on the machine-pass shape | does not exist in the repo at all; listed at KE12 |
| loom/Miri coverage for the `&self` lane path | promised by the design, not yet written |
| the `Or<(` blast radius behind KE1 ("~117 production sites") | **re-measured 2026-08-29: 112 textual matches**, and only **4** of them are production type positions. Producing command: `rg -n "Or<\(" -g "*.rs" crates/ src/` (line matches; run as the PowerShell `Select-String` equivalent on this checkout). Breakdown in [`KERNEL-BACKLOG.md`](KERNEL-BACKLOG.md) KE1 |
| probe blocks **C** and **E** — the red-first oracles AIR-04 and AIR-05 name | **measured in-session, worktree only — NOT committed.** No such fixture exists in the repo or in its history, so both oracles currently name an artifact that cannot be run. Promote each into `crates/aether_tests/tests/ui/` as an explicit R3 deliverable (probe C = a broken construct followed by `scen lab {}`; probe E = one block with two independent block-level defects; each pinning error COUNT == 2) |
