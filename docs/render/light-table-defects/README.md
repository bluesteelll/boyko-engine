# Light-table defects R1 / R2 (+ R2b, R3) — design record

Found 2026-09-18 by the boot-validation lane (`fix/boot-validation-errors`):

- **R1** — an un-slotted punctual light's light-table row carries atlas slot 0, not `SLOT_NONE`, so on
  a frame with the punctual atlas armed it samples another light's map. Fix: host-side, the row is
  born `SLOT_NONE`; no shader code, no `.spv`, no pin moves.
- **R2** — the Deferred SDF marcher is lit by a boot constant (`DEFAULT_SUN_DIR`), not the scene's
  primary directional light. Fix: host-side, the sun is read from the staged light table each frame;
  no sun ⇒ no sun shadow. `grand_showcase_2mat` moves by design (owner sign-off before re-bless).
- **R2b** — the CSM fit takes the first sun without the `LightEnabled` filter.
- **R3** — a test reads back the DDGI irradiance atlas, which was created without `TRANSFER_SRC`.

Status: **designed and reviewed (both APPROVED WITH CHANGES), not implemented.** One lane, after the
boot-validation lane merges: R1, then R2 (test-only commit, then the fix), R2b, R3.

| File | What it is |
|---|---|
| `00-RULINGS.md` | The orchestrator's rulings on both reviews — read this first |
| `R1-DESIGN.md`, `R1-REVIEW.md`, `R1-RESEARCH.md` | R1: plan, critique, tree + practice research |
| `R2-DESIGN.md`, `R2-REVIEW.md`, `R2-RESEARCH.md` | R2 (+ R2b): plan, critique, tree + practice research |

File:line citations point at `D:/wt/vkval` as of 2026-09-18; re-derive them at the lane's cut.
