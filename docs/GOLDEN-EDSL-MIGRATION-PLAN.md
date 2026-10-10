# Golden host-oracle → eDSL migration plan

**Goal.** Retire the hand-maintained CPU shader mirrors in
[`crates/boyko_rhi_vulkan/src/goldens.rs`](../crates/boyko_rhi_vulkan/src/goldens.rs)
(~4.2 kLOC, 43 `golden_*`/`host_*` functions) by **deriving** the host reference from the
same [`boyko_shaderdsl`](../crates/boyko_shaderdsl/) source that emits the HLSL — one source
of truth for the GPU shader *and* its CPU oracle, instead of two hand-synced copies that drift
on every shader edit.

**Owner decision (2026-07-09):** *full incremental migration* — one shader at a time, each
port gated Tier-0 byte-identical, verified by the owner's GPU golden run.

---

## The circularity guardrail (READ FIRST — this is why the order matters)

An oracle **derived from the same eDSL AST as the shader cannot catch a bug in that AST**:
a mistake in the eDSL source flows into both the HLSL and the derived oracle, so the
byte-identity test passes while *both* are wrong versus intent. The eDSL-derived oracle is
therefore only trustworthy when it is **independently anchored to real GPU output**.

**Hard precondition for migrating any shader:** an **independent GPU-readback golden** must
already cover it (real SPIR-V executed on the device, readback compared to a pinned value or
to a *separately-authored* reference). Only then may its hand-written `goldens.rs` mirror be
replaced by an eDSL-derived one — the GPU golden becomes the independent check that keeps the
derivation honest.

Shaders with **no** independent GPU golden are **not migratable yet**: author the GPU golden
first (a prerequisite work item), then migrate.

Corollary: **never delete a hand mirror in the same step that adds its eDSL-derived
replacement.** Land the derived oracle alongside the hand mirror, prove they agree bit-for-bit
AND that both agree with the GPU golden, and only then remove the hand copy — each step its own
Tier-0-gated commit.

---

## Current mirror inventory (from the 2026-07-09 infra map)

| Shader / construct | Host mirror today | Independent GPU golden? | Migratable now? |
|---|---|---|---|
| SSAO attributes + blur (`sdf_ssao*.comp.hlsl`) | `goldens.rs` + `ssao_edsl_sync` | yes (eDSL-authored, `ssao_edsl_sync`) | **YES — pilot candidate** |
| SDF field / edit-list (`sdf_editlist*.hlsl`) | `goldens.rs` composite/editlist family + `sdf_field_edsl_sync` | yes (`sdf_field_edsl_sync`, 23 tests) | **YES** |
| G-buffer MRT VS (`gbuffer_mrt.vs.hlsl`) | `instanced_vs_host_mirror.rs` + `gbuffer_mrt_edsl_sync` | yes (eDSL-authored) | **YES** |
| Interp instances (`interp_instances.comp.hlsl`) | `interp_edsl_sync` | yes (eDSL-authored) | **YES** |
| Deferred resolve / soft-shadow / AO / cluster-cull (`deferred_pbr.hlsl`) | `goldens.rs` (the bulk) | partial (composite readback) | **MODIFY — port the composite-covered parts only** |
| Lighting L0/L0b/L1 (`deferred_pbr` lighting) | `lighting_l0/l0b/l1_host_oracle.rs` (CPU-only) | **no** | **NO — author a GPU golden first** |
| DDGI probe sample (`ddgi_probe_gi_resolve.comp.hlsl`) | `ddgi_probe_sample_host_oracle.rs` (CPU-only) | **no** | **NO — author a GPU golden first** |
| CSM / punctual depth, TLAS build, atrous/temporal denoise, field-probe | none | varies (some `CHANNEL_TOL` readback only) | case-by-case |

The four `*_edsl_sync`-covered shaders (SSAO, SDF field, gbuffer_mrt, interp) are the safe
front of the migration: the eDSL already emits their HLSL, and a byte-identity sync test plus a
GPU readback already anchor them.

---

## The migration pattern (per shader)

1. **Confirm the anchor.** Verify an independent GPU-readback golden exists and is green. If
   not, STOP — file the "author GPU golden for `<shader>`" prerequisite and pick another shader.
2. **Emit the oracle from the eDSL.** Extend the shader's `boyko_shaderdsl` definition so the
   same AST can lower to a Rust reference function (a CPU evaluator), in addition to HLSL.
   Prefer a shared numeric core the eDSL and the emitter both call, so there is literally one
   arithmetic definition.
3. **Land alongside, prove agreement.** Add the derived oracle next to the existing hand mirror.
   Add a test asserting `derived == hand_mirror` bit-for-bit, and confirm the existing GPU
   golden still passes against the derived oracle.
4. **Owner GPU run (Tier-0).** `scripts\golden.ps1 -Check` (and `-Hwrt`) must stay byte-identical
   (`grand_showcase` + any per-shader pin). Subagents cannot run GPU exes — this leg is the
   owner/orchestrator's.
5. **Remove the hand mirror.** Only after 3+4 are green, delete the `goldens.rs` (or standalone
   `*_host_oracle.rs`) hand copy in a separate commit. Re-run the Tier-0 gate.
6. **Record.** Tick the row in this file; note the LOC retired.

Each shader is one or two commits, author-only, each Tier-0-gated. No big-bang.

---

## Phase order

- **P0 — pilot: SSAO.** eDSL-authored, `ssao_edsl_sync` + GPU goldens already anchor it, and
  its `goldens.rs` mirror (`golden_ssao_attributes` / `golden_ssao_blur`) is self-contained.
  Prove the whole pattern end-to-end on one shader before scaling. Deliverable: the eDSL→Rust
  oracle lowering + the `derived == hand` test + the retired hand mirror.
- **P1 — the rest of the `*_edsl_sync` front:** SDF field/edit-list, gbuffer_mrt VS, interp.
- **P2 — deferred_pbr composite-covered parts:** soft-shadow / AO / shade / cluster-cull, only
  where the composite readback golden anchors them.
- **P3 — prerequisite GPU goldens:** author independent GPU-readback goldens for lighting
  L0/L0b/L1 and DDGI probe-sample; then migrate those.
- **P4 — long tail:** CSM/punctual depth, TLAS build, atrous/temporal denoise, field-probe —
  each needs its own GPU golden authored first (P3 pattern), then migrate.

## Progress log

- 2026-07-09 — plan authored; owner approved full incremental migration. Tooling prerequisites
  (the Tier-0 gate [`scripts/golden.ps1`](../scripts/golden.ps1) + single-source
  [`goldens/PINS.toml`](../goldens/PINS.toml)) shipped in the same session. P0 not yet started.
- 2026-09-10 — **P0 steps 1-3 landed** on branch `feat/golden-edsl-p0` (commit `<hash>`).
  (1) `boyko_shaderdsl::ssao::{ssao_horizon_step_body_params, ssao_slice_body_params,
  ssao_estimate_body_params}` thread an `SsaoParams` preset through the generic bodies
  (emit-invisible: Emit prints the loop-bound SYMBOL and the named-literal SYMBOL, never their
  values; the legacy signatures delegate with the Medium row — `ssao_edsl_sync`'s
  `ssao_horizon_step_matches_edsl_emit` and both `.spv` byte-identity pins stayed green across the
  change). `boyko_rhi_vulkan` gains an OPTIONAL `boyko_shaderdsl` edge (Eval path only, no `emit`,
  `default-features = false`) armed by its `goldens` feature — note the eDSL was ALREADY in the
  crate's normal dependency graph transitively via `boyko_sdf_math`, so no new code reaches a
  shipped build that did not already; what is new is only the DIRECT edge, present with `goldens`
  and absent without it (`cargo tree -e normal --depth 1`, both ways). ⚠️ REVIEW ROUND 1: the
  `Cargo.toml` feature comment had asserted the OPPOSITE of this bullet — "a default/production
  build still links neither the oracles nor the eDSL" — while `cargo tree -p boyko_rhi_vulkan
  -e normal` with no features prints `boyko_sdf_math → boyko_shaderdsl` (an unconditional edge;
  `boyko_sdf_math/src/brick.rs:1073` delegates to `boyko_shaderdsl::brick::decode_snorm8`). The
  comment now states the measured property instead, with the `cargo tree` receipt inline.
  `goldens::golden_ssao_attributes_derived` instantiates `ssao_estimate_body_params::<EvalCf>`
  behind a TRANSCRIPTION (deliberately not a shared helper) of the hand-written seam glue. For
  SSAO no separate "eDSL→Rust lowering" exists to build: `<EvalCf>` IS the `f32` reference, so
  the derived oracle *instantiates* rather than *lowers*.
  (2) `crates/boyko_rhi_vulkan/tests/ssao_golden_derived.rs` (5 tests, device-free) proves
  derived == hand BIT-FOR-BIT (`to_bits`) over the GPU golden's exact inputs (the 3 P4b scenes ×
  3 presets × 64×64, e.g. crater: 2590 lit / 2352·2375·2428 occluded pixels per preset), plus
  border/seam/checkerboard/all-dark fixtures over every pixel and a PERSPECTIVE camera (the
  `pix_radius` clamp arm). ⚠️ MEASURED CAVEAT on that leg's apparent breadth: the three P4b
  scenes yield only TWO distinct occluded sets, not three — `crater_csg` and `smooth_union`
  have different G-buffers (different `view_t` hashes) yet an IDENTICAL 2375-pixel AO < 1 set,
  because the AO < 1 region is the shared mesh quad's silhouette and the two scenes' SDF
  geometry differs only where AO stays exactly 1.0. The horizon path's independent variety on
  this leg therefore comes from `box_csg` and, far more, from the synthetic fixtures
  (3865-4084 occluded pixels each). Step 5 should add an SDF-only (mesh-free) scene rather
  than assume "3 scenes" means three horizon neighbourhoods. The equivalence gate itself is
  unaffected — every mutation below reddens it at a P4b pixel — and it keeps permanent positive
  controls (lit > 0, occluded > 0, and the three
  presets distinguishable by BOTH oracles). Falsifiability was measured, not assumed: a one-ULP
  `next_up` on the derived lit result → the three equivalence tests red at the first occluded
  pixel of every fixture (`0x3f7db8f5` vs `0x3f7db8f6`), the preset-control green; a derived
  oracle that ignores its `params` (Medium shadow) → all four red, the control naming
  "`params` plumbing is dead"; a Medium-const step-loop bound in `ssao_slice_body_params` → the
  three equivalence tests red — but NOT where predicted: on the ORTHO P4b scenes the first red
  was `crater_csg / high`, because Low's extra 4th tap lands beyond `SSAO_RADIUS` except where
  pixel rounding pulls it inside, and the preset control stayed green because the seam's
  `advance` divisor still reads `params.steps`. Each mutation was restored by `cp` + `cmp`.
  ⚠️ **REVIEW ROUND 1 found the preset-driven legs cover only TWO of the five `SsaoParams`
  fields, and the gap was closed rather than argued away.** All three `SSAO_PARAMS` rows carry
  the SAME `radius` (0.5), `strength` (2.5) and `eps` (1.0e-4) — they differ only in `slices`
  and `steps` — so a derived oracle substituting the module const for any of those three passes
  every preset leg. MEASURED: with `radius: params.radius` in `golden_ssao_attributes_derived`'s
  `EdslSsaoParams` replaced by the literal `0.5`, the four original tests are `4 passed`. The
  fifth test `derived_matches_hand_mirror_under_off_table_params` closes it: both oracles take a
  plain `&SsaoParams`, so it drives them with a synthetic row off the table in all five fields
  (`radius 0.35, slices 4, steps 5, strength 1.7, eps 0.5`) plus each single-field variation of
  it, over the crater scene and the crevice fixture under BOTH camera arms. `eps` is off-table
  LARGE on purpose: `elev = dot(delta,N)/max(length(delta), eps)` reads `eps` only when it is
  the max, and the fixtures' tap deltas are ~1e-2..3.5e-1, so a small off-table `eps` would be
  unobservable and the leg would silently not cover the field. RED-FIRST, three separate
  mutations, each the table value substituted for the field, each restored by `cp` + `cmp`:
  `radius → 0.5`, `strength → 2.5`, `eps → 1.0e-4` — every one gives
  `test result: FAILED. 4 passed; 1 failed`, the single failure being the new leg at
  `[p4b crater_csg ortho / base] pixel (0,0)` (hand `0x3f7fc54f` vs derived `0x3f7fab5d` /
  `0x3f7fa9b5` / `0x3f7f2b04`). The leg also carries a per-field OBSERVABILITY control (moving
  each field alone must change BOTH oracles' images), so it cannot go quietly blind if a fixture
  is later retuned.
  (3) `ssao_edsl_sync` (6 tests) and `sdf_gbuffer_hybrid` are byte-unchanged; the former is green,
  the latter compiles (its GPU run is step 4). **The hand mirror is NOT deleted.** Step 4 (owner:
  `scripts/golden.ps1 -Check`) and step 5 (delete `golden_ssao_attributes` + `ssao_horizon_step`,
  swap `sdf_gbuffer_hybrid.rs`'s call sites, hoist the P4b fixtures into `tests/common`, and
  correct `ssao_edsl_sync.rs`'s two sentences that say the lib does not link the eDSL) are NOT
  started. On that last item — `ssao_edsl_sync.rs:357` ("`boyko_shaderdsl` is a DEV-dependency
  only — the lib must not link the eDSL") and `:450` ("the lib has no dev-dep on it") — note the
  correction is NOT merely "add the `goldens` feature": both sentences were ALREADY imprecise
  before this branch, for the transitive reason recorded above (`boyko_sdf_math` links
  `boyko_shaderdsl` unconditionally in every shipped build). The true property they were
  reaching for is "the shipped `compute` surface never NAMES the eDSL". The file stays
  byte-unchanged in P0 because it is the drift gate this branch must not perturb.
