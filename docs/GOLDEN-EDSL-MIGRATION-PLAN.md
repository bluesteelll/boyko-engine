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
