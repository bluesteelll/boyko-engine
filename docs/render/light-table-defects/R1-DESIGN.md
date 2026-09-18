# R1 fix plan: un-slotted punctual lights get `SLOT_NONE` when the row is built

## Summary

- **The defect is entirely on the host side.** There are six shader sites, not three. All six check `light_atlas_slot(L.kind) != SLOT_NONE`, and all six are already correct for a row that carries the `0x1F` value. The rows don't carry it:
  - `GpuLight::from_point` and `from_spot` (`crates/boyko_render/src/light.rs:1327`, `:1350`) write slot field 0.
  - `slot_pack` (`light_system.rs:451-456`) skips the pack when the base is `SLOT_NONE`.
- **The fix:** build every point/spot kind word as `KIND | SLOT_NONE_FIELD`, where `SLOT_NONE_FIELD = SLOT_NONE << ATLAS_SLOT_SHIFT` = `0x003E_0000`. Do the same in `GoldenLight::point`/`spot`. `slot_pack` keeps its guard.
- **What it leaves alone:**
  - No shader code changes, no `.spv` is re-emitted, and no `SHADER-VARIANT-MANIFEST` row is touched.
  - No pin moves on either leg.
  - No instruction is added anywhere.
- **Correction to the brief:** `vb_mesh_froxel` and `vb_mesh_tex_froxel` are not exposed.
  - Neither scene spawns a `ShadowCaster` (`crates/boyko_app/tests/vb_mesh_froxel.rs:206` spawns a plain `MeshBundle`). So `depth_pass_armed` (`shadow_atlas.rs:310-312`) is false and header bit 3 never arms.
  - No pinned scene is exposed. The one exposed scene is `crates/boyko_app/examples/vb_lab.rs`, which is not pinned.
  - After the fix, the golden run itself checks this on the device (see section 3).

## Decision: set the value when the row is built, leave the shaders alone

**Why this is the best option:**
- The one decoder of the kind word is `load_light` (`light_table.hlsli:291`).
- Every kind comparison masks the word with `0xFFFF`: all shader files go through `light_kind()`, and the CPU oracles go through `GoldenLight::kind()`. So the only thing that reads the slot field is the six sites, and they read it only under bit 3.
- A row that carries `0x1F` is therefore rejected by code that already exists, on every path and variant, on both the software and hwrt legs.
- Setting the value when the row is built is what URP does (fill every entry with the default before assigning real slices). The "no map" state becomes the default: `from_*` produces it, and `slot_pack` only ever adds a real assignment on top. Leaving the guard in place means the value has exactly one owner, which makes that owner something a mutation can target.

| Alternative | Rejected because |
|---|---|
| **B.** The six consumers also test bit 16 | Costs 2 more ALU ops per light per pixel on armed frames, edits 6 hand-written sites, and forces 20 `.spv` to be re-emitted and their byte gates re-blessed. It also **does not fix the class**: `GoldenLight::with_sdf_shadow()` sets bit 16 with no slot, so a fixture with bit 3 armed still samples layer 0. And it ties the atlas to a bit whose shader meaning is "casts an SDF shadow" (`light_table.hlsli:56`). |
| **C.** Store `slot+1` in the field, so 0 means "none" | Safe even for zero-filled words, but costs 1 IADD per light-iteration on armed frames, 20 `.spv` re-emitted and re-blessed, and a new encoding for every slotted row too. The benefit can't be reached: rows are only ever built by `from_*` / `GoldenLight::*`, which this fix covers. The zero-filled tail of `LightTableStaging` is never read, because uploads cover `[..used_bytes)` and the shaders stop at the header counts. |
| **Y.** Make the fold own the value (always pack, keep `from_*` as is) | Any `GpuLight` built outside the fold would still decode as slot 0. And the value would have two possible owners, so no single mutation could put the defect back. |

**What it costs:** the bytes of every un-slotted punctual row change, from `0x0000_000k` to `0x003E_000k`. That ends the "un-slotted table is byte-identical to the pre-Inc-1 fold" property, on purpose: in the Inc-1 encoding, the pre-Inc-1 word *means* "slot 0", so that identity was the defect. The pixel 0%-gate still holds, and that is what the pins check. Also, bit 16 still means "slotted" on the host (`pack_atlas_slot` is unchanged), so the poison probe and the arming test keep working.

## 1. Changes (files under D:/wt/vkval)

No eDSL-owned HLSL is touched, no `.spv` is re-emitted, and no manifest row changes.

| File | Change |
|---|---|
| `crates/boyko_render/src/shadow_atlas.rs` | After `:76`, add `pub const SLOT_NONE_FIELD: u32 = SLOT_NONE << ATLAS_SLOT_SHIFT;`. Near `:137`, add a `const` assert that it does not overlap `0xFFFF \| CASTS_SHADOW_BIT`. Fix the docs listed in section 5. |
| `crates/boyko_render/src/lib.rs:609-611` | Re-export `SLOT_NONE_FIELD`. |
| `crates/boyko_render/src/light.rs` | `:1327` becomes `f32::from_bits(LIGHT_KIND_POINT \| SLOT_NONE_FIELD)`, and `:1350` does the same for `LIGHT_KIND_SPOT`. Update docs at `:105-107`, `:118`, `:1320-1322`, `:1338-1341`. Tests `:1871` and `:1884` expect `KIND \| SLOT_NONE_FIELD`. |
| `crates/boyko_render/src/light_system.rs` | The code at `:451-456` is unchanged (keep the guard). Fix the docs in section 5. Add gate G1; rewrite the tests at `:1218-1249` and delete `:1251-1279` (details in section 4). |
| `crates/boyko_rhi_vulkan/src/compute.rs` | After `:5458`, add `GOLDEN_SLOT_NONE_FIELD`. Update the `:5463-5465` doc: the host also sets bit 16 on slotted rows. |
| `crates/boyko_rhi_vulkan/src/goldens.rs` | `:1313` and `:1341` build `KIND \| GOLDEN_SLOT_NONE_FIELD`. Add the import at `:27-29`. Fix docs at `:1305`, `:1319`, `:1377-1384`. |
| `crates/boyko_rhi_vulkan/shaders/light_table.hlsli` | Comment-only edit at `:215-224` and `:56` (section 5). Without `-Zi` the SPIR-V bytes cannot change. The existing re-DXC `*_spv_sync` gates must stay green **without re-blessing**. If one goes red, the edit was not comment-only: revert it. |
| `crates/boyko_app/src/shadow_poison.rs` | `staged_shadow_words` (`:158-174`) returns `(word7, slotted, sampled)`. `sampled` counts rows where `kind & 0xFFFF ∈ {POINT, SPOT}` **and** `light_atlas_slot(kind) != SLOT_NONE`, which is the shader's own predicate. The kind filter is required because directional and sky rows keep field 0. Add `sampled_rows` to `ShadowProbeRecord` (`:137-140`) and to `format_record` (`:193-208`). Update the unit test at `:319-326`. |
| `crates/boyko_app/src/runner.rs:3389-3407` | Unpack the triple and pass `sampled_rows` through. |
| `crates/boyko_render/tests/mesh_shadow_arming_agreement.rs`, `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs`, `crates/boyko_app/tests/unwritten_shadow_map_gate.rs` | Gates G2, G3 and U (section 4). |

## 2. Hot-path cost

- **Shaders:** no instructions added. The existing gate (`ubfe`, `ine`, branch per light per pixel, only under bit 3) is unchanged.
  - On an armed frame, each un-slotted light used to pay a spot matrix-vector multiply or a point face select, plus the 13-tap `atlas_pcf_disc`. It now skips all of that.
  - So the fix is a net **saving** of one PCF per un-slotted light per covered pixel (in `vb_lab`, one PCF per pixel in the blue point's range).
- **Host:** `KIND | SLOT_NONE_FIELD` is a constant expression, so the generated code is the same single constant store. The fold's code is unchanged. It runs only when the table is rebuilt.
- **No benchmark:** no host or shader code path changes, so a benchmark would measure nothing. The only runtime change is the skipped PCF.

## 3. What should happen to the images

- **Pinned goldens, software and hwrt legs: none move**, and that includes the two lane pins still marked `PENDING`.
  - Tables built from `GoldenLight` with bit 3 armed (`grand_showcase`, `deferred_sdf_only`, `deferred_mesh_only`) have one punctual light each, and `with_atlas_slot(0)` clears and rewrites its field. Their input bytes are identical.
  - ECS scenes with point or spot lights (`vb_mesh_froxel`, `vb_mesh_tex_froxel`, `room_smoke*`) never set bit 3, so the slot field goes unread. The froxel pins' equality contract with `BOYKO_VB_FROXEL_FORCE_OFF` still holds.
- **If either froxel pin moves:** that disproves "bit 3 never arms" and supports the brief. Stop, read `punctual_armed=` from the runner's dump-stats line (`runner.rs:3590`), and do not re-bless.
- **`examples/vb_lab.rs` (not pinned; for you to check by eye):**
  - Its cool-blue point accent at (-1.8, 2.2, 2.4) is un-flagged. Today it samples atlas faces using the **spot's** `light_pos` and `inv_range` (`gFaces[0]`), on layers 1..5, which were never rendered.
  - **Before the fix:** the blue fill on the floor and props can be missing or blotchy in wedges whose straight boundaries meet at the *spot's* position (3.6, 4.2, 3.2). Exactly what appears depends on what the unwritten layers hold.
  - **After the fix:** a smooth blue fill with no shadow of its own, which is what the source comment ("unshadowed") says it should be.
- **Gate artifacts** (`temp_dir()/boyko_unwritten_shadow_map_gate/*_pu_*.bmp`):
  - **Before the fix:** the orange point's light is black over most of the floor and sphere at poison 0.0, and lit at poison 1.0.
  - **After the fix:** the two images are identical, with the point unshadowed.

## 4. Gates

Every gate is added first and run red on today's tree; then the fix goes in.

**Device-free gates (the main ones, because the property now lives on the host):**

- **G1** — `light_system.rs` tests, new `every_punctual_row_decodes_exactly_its_assignment`.
  - Setup: 3 points and 3 spots. All 64 assignment masks, each winner given a distinct real base (points ≤ `M_SLOTS - POINT_FACE_COUNT`), folded through `fold_light_table_slotted` using `base_for`.
  - Checks, per row:
    - `light_atlas_slot(k) == base_for(id)`
    - `(k & CASTS_SHADOW_BIT != 0) == (base != SLOT_NONE)`
    - `k & 0xFFFF == KIND`
    - no bits outside the kind, bit 16 and the slot field
    - the whole word equals a **literal** computed without `pack_atlas_slot` (`0x003E_0001` / `0x003E_0002` for un-slotted rows)
  - Red today at mask 0: all six rows decode 0.
  - Also: `slotted_fold_loser_packs_slot_none_not_zero` gets assertions that match its name (`== SLOT_NONE`, `== 0x003E_0002`) and a corrected comment. Delete the circular `slotted_fold_no_assignment_is_byte_identical_to_unslotted` (it compares the fold with itself); G1's mask 0 replaces it.
- **G2** — `mesh_shadow_arming_agreement.rs`: add an **un-flagged** point to the scene at `:129-147`.
  - For every one of the 24 configs, check every staged point/spot row: `(k & CASTS_SHADOW_BIT != 0) == (light_atlas_slot(k) != SLOT_NONE)`.
  - Guard against a vacuous pass: every config must have at least one punctual row whose field is `SLOT_NONE`.
  - Red today in all 24 configs, on the un-flagged point, and in the 12 mesh-less configs on both flagged lights too. This runs through the real ECS pipeline: resolve, then assignment, then `collect_lights`.
- **G3** — `lighting_l1_host_oracle.rs` (CPU-only), new `every_light_row_producer_emits_the_slot_none_sentinel`. It checks:
  - (a) `GoldenLight::point`/`spot` raw words equal `0x003E_0001` / `0x003E_0002`, with bit 16 clear.
  - (b) `boyko_render::GpuLight::from_point`/`from_spot` produce the same words (mirror parity).
  - (c) `light_table.hlsli`, whitespace-normalised, defines `ATLAS_SLOT_SHIFT = 17u`, `ATLAS_SLOT_MASK = 0x1Fu`, `SLOT_NONE = 0x1Fu` and `return (kind_word >> ATLAS_SLOT_SHIFT) & ATLAS_SLOT_MASK;`, each equal to the Rust constant. This is the only device-free link between the host value and the shader predicate.
  - Red today on (a) and (b).

**Device gate** (extends `unwritten_shadow_map_gate.rs`; R1's point case is a read of a never-written layer, which is what that file tests):

- **Worker:** a required `BOYKO_SHADOW_GATE_SCENE` variable with three values:
  - `flagged` — today's scene; the existing drivers set it.
  - `point_unflagged` — the point spawned without `CastsPunctualShadow`.
  - `no_point`

  `RunSpec` and `label()` carry the scene.
- **Existing tests:**
  - I4 additionally requires `sampled_rows == 0`. It is red today: under the EMPTY handoff both flagged rows decode 0.
  - C3 additionally requires `sampled_rows == slotted_rows`.
- **New test `unslotted_punctual_lights_never_sample_the_atlas`:**
  - Runs: for P in {deferred, forward, forwardplus, vb} × Both, u0 (poison 0.0) and u1 (poison 1.0). For P in {deferred, vb}, one n0 run with `no_point`. **Exactly 10 runs.**
  - **U1:** frame(u0) == frame(u1).
  - **U2:** each u-run has `punctual_armed_frames > 0`, header bit 3 set, `atlas_active_layers == 1`, `slotted_rows == 1`, and `sampled_rows == 1`.
  - **U3:** in both u-runs, atlas centre texels `[1..16)` equal the poison bits (that is where the buggy read lands), and `atlas[0]` in u0 is not the 0.0 poison.
  - **U5:** frame(u0) != frame(n0), which proves the point lights visible pixels, so U1 can fail.
  - Expected today: U1 red on all four paths, U2 red (`sampled_rows` 2).
  - The un-slotted **spot** reads layer 0, which has been written, so the poison cannot show it. G1 and G2 cover spots, since both kinds go through the same predicate line.

**Mutation receipts after the fix:**

| Mutation | Should turn red |
|---|---|
| R1-M1: `from_point` back to the raw kind | G1, G2, G3, U1, U2, I4 |
| R1-M2: `from_spot` back to the raw kind | G1, G2 (mesh-less configs), G3, I4 |
| R1-M3: `GoldenLight::point`/`spot` back to the raw kind | G3 |
| R1-M4: delete the `slot_pack` guard | Must stay **green** (the value's single owner is where the row is built) |

## 5. Docs and comments that are false and must be corrected

- `light_table.hlsli:223-224`
  - Its claim that the slot is read "AND `casts_shadow`" is false.
  - New text: a row is born `SLOT_NONE`, and the shader tests only bit 3 and the field. Add at `:56` that the host also sets bit 16 on slotted rows.
- `shadow_atlas.rs`:
  - `:78-81` — "the resolve tests it".
  - `:404-406` — "byte-identical to the pre-wiring path".
  - `:464-466` — the fallback is caused by the field, not by bit 16.
  - `:1116-1135` — rewrite: the second check now rejects un-slotted rows. Delete case 2's clause about un-slotted rows; case 3 applies only to slotted rows.
- `light_system.rs`:
  - `:233-236`, `:247-253`, `:445-449` (the guard now skips a no-op; it no longer "preserves byte-identity").
  - `:628-633`, `:1218-1220`, `:1244-1246`.
- `mesh_shadow_arming_agreement.rs:25-28` — "its slot field is 0, not SLOT_NONE".
- `goldens.rs:1380-1381` — "so the resolve branches onto the map sample".
- `docs/OPEN-QUESTIONS.md:21-68`
  - Close the entry.
  - Its exposure claim at `:49-52` is wrong for the reason above.
  - Its "changes shaders, `.spv` and pins" at `:25` and `:52` does not apply to this design.

## 6. What the fix must NOT do

- Must not edit any HLSL code, re-emit any `.spv` (up to 20 carry the sites), or touch any `SHADER-VARIANT-MANIFEST` row.
- Must not change the slot field's position or width (shift 17, mask `0x1F`, `SLOT_NONE`), or bit 16 ⇔ slotted on slotted rows. `shadow_poison`, the arming test, `with_atlas_slot` and `lighting_l1_host_oracle.rs:633` depend on it.
- Must not change the kind tag (bits 0..15), or the directional and sky rows (`from_directional` / `from_sky`); nothing reads their slot field.
- Must not remove the `slot_pack` guard.
- Must not move any pin, or re-bless one to make a run pass.
- Must not touch SG1–SG5 (`resolve_shadow_atlas`, `sync_punctual_light_gate`, `frame_uniform`) or the F2 residual.
- Must not touch the second meaning of bit 3 in `deferred_pbr.hlsl:1371` (`vis = armed ? 1 : shadow`, open owner decision F3). An un-slotted light on an armed Deferred frame keeps `vis = 1.0`; that belongs to F3, not R1.
- Must not compare kind words as `f32`; the new words are subnormal, so compare `to_bits()`.

**Out of scope, named follow-up R1-F1:** bit 16 means one thing to the host (slotted) and another to the shader (casts an SDF shadow). It moves no pixel today and can only be reached through header bit 0, which has no production writer (`light.rs:401-402`). I made this call on those grounds; it is not a decision for the owner.

## Order

1. Add G1–G3, the probe field, U, and the I4/C3 additions. Run red on today's tree, device-free and on the device.
2. Add the value where rows are built: `shadow_atlas.rs`, `light.rs`, `compute.rs`, `goldens.rs`.
3. Correct the docs.
4. Run `cargo test --workspace --all-targets --no-fail-fast` (`spv_sync` must be green without re-blessing), `scripts/golden.ps1` on both legs (nothing moves), and the device gate (green).
5. Run R1-M1..M4.

There is no threading change: `collect_lights` is still the single writer.