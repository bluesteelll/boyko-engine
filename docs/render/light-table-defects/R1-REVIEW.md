# Architecture review: R1 fix. Un-slotted punctual rows are born `SLOT_NONE`

## Verdict
**APPROVED WITH CHANGES.** There are no Blocking or Important remarks. The changes listed are small corrections to the plan's own receipts and doc list, plus two cheap gate improvements. None of them changes the design.

## What I verified against D:/wt/vkval (all CONFIRMED)

- **Six sites, all gated by header bit 3 and only by `slot != SLOT_NONE`:**
  - `deferred_pbr.hlsl:1332-1334`
  - `forward_opaque.fs.hlsl:414-416`
  - `sdf_forward_march.comp.hlsl:1108-1110`
  - `vb_resolve.comp.hlsl:464-466`
  - `vb_shade.comp.hlsl:619-621`
  - `vb_shade_split.comp.hlsl:586-588`
  - No HWRT variant touches the punctual loop. The `#if HWRT` arms at `deferred_pbr.hlsl:1001` and `:1403` sit outside it, and `vb_shadow_vis`/`sdf_probe_update` are directional-only.
- **Every other kind consumer masks the word:**
  - Every shader `kind ==` goes through `light_kind()`: `cluster_cull.hlsl:323,356,475`, all resolve and forward files, and `emit_probe_gi.rs:345`.
  - `light_casts_sdf_shadow` reads only bit 16, which `SLOT_NONE_FIELD` (`0x003E_0000`) does not touch.
  - The CPU oracles use `GoldenLight::kind()` (`goldens.rs:3100,3345,3602,3909,3939,4099,4310`).
  - No test pins raw point/spot kind words other than the ones the plan already lists (`light.rs:1871,1884`, `light_system.rs:1248`, `shadow_poison.rs:323`).
  - No DAZ/FTZ setting exists anywhere. Also, the old words `0x1` and `0x2` were already subnormal, so the new subnormal words add no risk.
- **Rows are built only by `from_*`, and only inside the fold.** `from_point`/`from_spot` are called only at `light_system.rs:323,339` plus unit tests. `gpu_scene/mod.rs:698` packs only the empty boot seed.
- **No pin moves:**
  - The only pinned ECS scenes that enable `ShadowConfig` are `vb_mesh_froxel.rs:321` and `vb_mesh_tex_froxel.rs:340`. Neither has a `ShadowCaster`: `MeshBundle` (`bundles.rs:43-58`) carries none, and neither file inserts one. So `depth_pass_armed` (`shadow_atlas.rs:310-312`) is false and bit 3 never arms. The brief's exposure claim is refuted.
  - The GoldenLight fixtures that arm bit 3 (`window_present_gbuffer.rs:5466`, `:5631`, `:5879`) each hold one punctual light through `with_atlas_slot(0)`. That call clears and rewrites the field (`goldens.rs:1391-1400`), so their bytes are identical.
  - The TAA pins hand-seed only `csm_shadows` (`taa_jitter_eval.rs:530`), not bit 3.
  - `vb_lab.rs:308-312` is the only exposed scene. `showcase`, `punctual_room` and `boot_validation_clean` flag every light, and `paradigm_lab` flags none, so its `mode_word` stays 0.
- **Comment-only HLSL edit:**
  - `light_table.hlsli` has no GENERATED sentinel.
  - No recipe uses `-Zi`/`-fspv-debug` (the only `OpSource` in the tree is a bare `OpSource HLSL 600`).
  - No test reads or hashes that file.
  - The anchor-gated docs cite none of the edited files past the insertion points.
- **The plan's specific findings hold:**
  - `slotted_fold_no_assignment_is_byte_identical_to_unslotted` (`light_system.rs:1255-1279`) is circular: `fold_light_table` is `fold_light_table_slotted` with `SLOT_NONE` (`:237-244`).
  - `slotted_fold_loser_packs_slot_none_not_zero` asserts the opposite of its own name (`:1248`).
  - `light_table.hlsli:223-224` is false.
- **The boundary-crossing gate is buildable.** G3(b) can compile because `boyko_rhi_vulkan` already has `boyko-render` as a dev-dependency (`Cargo.toml:135`).

## Remarks

### Blocking
None.

### Important
None.

### Optional (the "changes" in the verdict)

**O1. G2's expected-red count is wrong.**
- **Where:** §4 G2, "in the 12 mesh-less configs on both flagged lights too".
- **Problem:** The flagged rows are un-slotted only when the resolve publishes EMPTY. That happens when `!mesh_shadow_producers()` (`shadow_atlas.rs:974`, which is `mesh_leg`, `render_path_config.rs:729`). That covers the Sdf leg only: 4 paths × 2 caster counts = **8** configs, not 12. With 0 casters and a mesh leg, the lights are still slotted.
- **Consequence:** A developer checking the red-first receipt will count 8 and may conclude the gate is wired wrong. The same "12" should become "8" in R1-M2's row.
- **Related, trivial:** §3 lists `deferred_sdf_only` as a table "with bit 3 armed". It is not: `window_present_gbuffer.rs:9040` filters `spot_atlas` by `mesh_leg`, and the pin's own comment says neither bit arms. This does not change the conclusion.
- **Confidence:** CONFIRMED.

**O2. §5 misses two doc sites that the fix makes false.**
- **`shadow_atlas.rs:976-978`** says every punctual row's `dir_kind.w` "stays byte-identical to the pre-wiring path". After the fix that is exactly the identity that ends.
- **`shadow_atlas.rs:945-950`** points readers at "What a punctual sample can read" for "why the header bit is the check that holds for an un-slotted row". The plan rewrites that section so that the slot field now also holds.
- **Consequence:** Two stale rationale comments survive a change whose §5 exists to remove every one of them.
- **Confidence:** CONFIRMED.

**O3. Two gate additions have no receipt, which the lane's rule requires.**
- **G3(c)** (the HLSL constant text-match) is green today and appears in no mutation row. Give it a negative control, for example running the matcher against an in-memory copy with `SLOT_NONE = 0x1Eu`, or name a mutation. Otherwise a normalisation mismatch can only surface as a panic that was never seen once.
- **The C3 addition** (`sampled_rows == slotted_rows`) runs only on the `flagged` scene, where every punctual row is slotted. None of R1-M1 to M3 can turn it red. Either give it a receipt or state that it is a consistency check and not an R1 gate.
- **Confidence:** CONFIRMED.

**O4. Add an un-flagged spot to G2, and optionally a budget loser.**
- **Why:** The device gate admits it cannot see the spot case, because the spot reads layer 0, which was written. Today, from_spot regressions (R1-M2) show up through the real resolve → assignment → `collect_lights` pipeline in the Sdf configs only.
- **Fix:** One un-flagged spot in `mesh_shadow_arming_agreement.rs:129-147` makes R1-M2 red in all 24 configs, including the armed ones. Two or three flagged points (more than 16 layers) would add a real slot loser through the production resolve. G1 currently covers that case only as a pure fold.
- **Confidence:** CONFIRMED (gap). Low cost.

**O5. Close the OPEN-QUESTIONS entry by the file's convention.**
- `docs/OPEN-QUESTIONS.md:10-12` requires marking an entry `RESOLVED` with a date, not deleting it.
- Record that the "not decided" fork at `:57-63` was taken as an architecture call, and why. The value of the record is its reasoning.

**O6. The fix adds a module cycle.**
- `light.rs` gains `use crate::shadow_atlas::SLOT_NONE_FIELD`, while `shadow_atlas.rs:46` already imports `crate::light`.
- This is legal and costs nothing at runtime. But after the fix, the kind-word layout lives in two modules, and the row constructor depends on the shadow policy module.
- Either note this, or put `SLOT_NONE_FIELD` next to `GpuLight` with `shadow_atlas` re-using it. Not a blocker.

## Positive (keep these)
- **The host-side fix at row construction is optimal.** It adds no ALU and changes no `.spv` or manifest row. It makes an un-slotted row self-describing through the sentinel the encoding already defined, and it removes a PCF per un-slotted light on armed frames.
- **The rejection of B is correct and non-obvious.** `with_sdf_shadow()` sets bit 16 without a slot (`goldens.rs:1371-1374`), so B would not fix the class.
- **Y and C are rejected for the right reasons:**
  - C: the staging tail is never read, because uploads cover `[..used_bytes)` (`light_system.rs:136-138`) and the shaders stop at the header counts.
  - Y: two owners would make the defect impossible to reintroduce by any single mutation, so no gate could be shown able to fail.
- **Keeping the `slot_pack` guard, with R1-M4 green, is a good demonstration** that the value has a single owner.
- **The gates are well designed:**
  - G1 compares against literals computed without `pack_atlas_slot`.
  - G2 has its anti-vacuity guard.
  - U1, U3 and U5 are well paired: U3 proves the poison reached the layers the buggy read lands on.
  - The plan states honestly that the device gate is blind to the spot case.
  - It gives a stop-and-read rule if a froxel pin moves.
- **Scoping is correct.** It fences off F3 (`deferred_pbr.hlsl:1371`) and R1-F1 (bit 16's two meanings, reachable only via header bit 0, which has no production writer, `light.rs:401-402`).

## Open questions for the architect
1. **U runs on Forward and ForwardPlus × Both, but C1 (a frame that does not depend on the poison under an armed flagged scene) has only ever been measured on Deferred and VB** (`unwritten_shadow_map_gate.rs:700`). If either forward path has an unrelated poison dependence, for example in the cascades, U1 is red before the fix for a mixed reason and stays red after it. Would you add the `flagged` d/e pair for `forward`/`forwardplus` (4 runs) so a U1 result there is attributable to R1? The Forward atlas pass is recorded (`graph_bridge.rs:2808-2816`, `passes/forward.rs:593`), so this is PLAUSIBLE rather than expected.
2. **U3 on armed frames** depends on the `atlas_depth` access over the full array (`SubRange::depth_layers(MAX_TEXTURE_LAYERS)`, `graph_bridge.rs:2810-2816`) not discarding layers 1..15. I3's "the driver preserves them" caveat was measured on the unarmed path only. The red-first U1 run settles this on the development machine. Please confirm that a green U1 today would count as "stop", like the froxel-pin rule.

Key files:
- D:/wt/vkval/crates/boyko_render/src/light.rs
- D:/wt/vkval/crates/boyko_render/src/light_system.rs
- D:/wt/vkval/crates/boyko_render/src/shadow_atlas.rs
- D:/wt/vkval/crates/boyko_rhi_vulkan/src/goldens.rs
- D:/wt/vkval/crates/boyko_rhi_vulkan/src/compute.rs
- D:/wt/vkval/crates/boyko_rhi_vulkan/shaders/light_table.hlsli
- D:/wt/vkval/crates/boyko_app/src/shadow_poison.rs
- D:/wt/vkval/crates/boyko_app/tests/unwritten_shadow_map_gate.rs
- D:/wt/vkval/crates/boyko_render/tests/mesh_shadow_arming_agreement.rs
- D:/wt/vkval/docs/OPEN-QUESTIONS.md