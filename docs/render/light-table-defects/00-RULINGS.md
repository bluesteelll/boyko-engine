# Orchestrator rulings on the R1 / R2 designs (2026-09-18)

Both designs APPROVED WITH CHANGES by their critics, no Blocking remarks. Implementation is ONE lane,
started from the integration line after `fix/boot-validation-errors` (vkval) has merged, in this order:
R1 commit(s), then R2 test-only commit, R2 fix commit, R2b commit. One worktree, so the shared files
(light.rs, light_system.rs, runner.rs, compute.rs) never conflict and the GPU is used by one run at a time.

## R1 — un-slotted punctual rows are born SLOT_NONE (R1-DESIGN.md + R1-REVIEW.md)

Accepted: host-side fix at row construction; no HLSL code change, no .spv re-emit, no pin moves.
Adopt every critic Optional:
- O1: G2's expected red is 8 mesh-less configs (Sdf leg), not 12; also drop the claim that
  `deferred_sdf_only` arms bit 3.
- O2: add `shadow_atlas.rs:976-978` and `:945-950` to the doc list.
- O3: G3(c) gets a negative control (matcher run on an in-memory copy with `SLOT_NONE = 0x1Eu` must
  fail); the C3 addition is labelled a consistency check, not an R1 gate.
- O4: add an un-flagged SPOT to G2's scene, and a budget loser through the production resolve
  (flagged points exceeding 16 layers).
- O5: close the OPEN-QUESTIONS entry as RESOLVED with the date and the reasoning; record that the
  froxel-pin exposure claim was refuted by reading (no ShadowCaster ⇒ bit 3 never arms) and that the
  only exposed scene is `examples/vb_lab.rs`.
- O6: define `SLOT_NONE_FIELD` next to `GpuLight` in light.rs and have shadow_atlas re-use it (no new
  module cycle).
Critic's open questions: (1) YES — add the `flagged` d/e pair for forward/forwardplus (4 runs) so a U1
red there is attributable to R1. (2) YES — a GREEN U1 on today's tree is a STOP, like a moved froxel
pin: report, do not proceed.
vb_lab before/after screenshots go to the owner (unpinned; for the eye only).

## R2 — the Deferred marcher takes the table's primary sun (R2-DESIGN.md + R2-REVIEW.md)

Accepted: host derivation from the staged table bytes; D2 (no sun ⇒ clear SHADOWS, keep AO, NaN guard);
D3 (dead Forward/VB push field stays; refactor last).
Required changes:
- W1: G3 uses the worker re-exec pattern (one binary, one scene per process), like
  unwritten_shadow_map_gate.rs / boot_validation_clean.rs.
- W2: G3 thresholds come from CONTROL RUNS in the same build (sun off; occluder removed), not guessed
  constants; the fixture states its light set (no SkyLight unless a control run makes the threshold
  independent of it) and the measurement space (the BMP is gamma-encoded after ACES).
- W3: `deferred_sdf_casters` is in the `taa_*` class (sun = the constant ⇒ same up to rounding; A/B
  before any re-bless). R2 starts after vkval's bless of it.
Optional, all adopted: use `golden.ps1 -Pin <name> [-Hwrt]` (no `-Check`); state acceptance rule 2 on
luminance (ACES off-diagonals can lower one channel) and rule 3 as an L1 distance over the changed mask;
a second, dimmer directional in T10 (dark spot stays under the first); T12's oracle is the ECS query
order, stated; repair the stale `f6147f90` citations after the re-bless; list the unpinned owner
reference dumps that will change (pbr_material_showcase, pbr_showcase, textured_smoke,
grand_showcase_mvpm); record sibling defect R2c in OPEN-QUESTIONS (on sunlit frames point/spot lights
and extra directionals take the PRIMARY sun's shadow mask, deferred_pbr.hlsl:1371 / :982) — not R2's.
`grand_showcase_2mat` moves by design (5 crescents, ~10^3 px): owner visual sign-off before bless.
R2b (CSM takes the first sun without the LightEnabled filter) is a separate commit in the same lane.

## R3 — a test readback of an image created without TRANSFER_SRC (vkval round 4, sweep row R9)

`crates/boyko_rhi_vulkan/tests/ddgi_probe_gi_arm.rs:342` copies the DDGI irradiance atlas to a buffer;
the atlas is created at `ddgi.rs:202` with `SAMPLED|TRANSFER_DST[|STORAGE]`, no TRANSFER_SRC
(VUID-vkCmdCopyImageToBuffer-srcImage-00186). The test boots with validation off, so nothing reports it.
Ruling: fix it on the TEST side — a readback-capable constructor (an extra-usage parameter or a
`create_for_readback`) that only the test uses; production usage stays as it is (a boot-created image
is not widened for a test). Arm the test's validation, or state why it cannot be armed. Pixel-neutral;
golden sweep not needed for a test-only construction path, but the test must go green under validation.
Same lane, its own small commit.
