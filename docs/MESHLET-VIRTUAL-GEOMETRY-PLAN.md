# VG-R0 — "The Ruler": the measurement rung of the virtual-geometry campaign

**Status:** DESIGN, **Rev 3** — **NOT APPROVED, and no code exists.** This document specifies
**only rung R0** of the ladder in [`docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`](MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md)
§4. R1–R8 stay as that document leaves them and are out of scope here. The owner's decision to
build a meshlet / virtual-geometry system is **settled** and is not re-litigated below.

**Revision history, kept because the errors are the useful part.** Rev 1 carried one open P0 and
three defects of one family — *a gate that cannot go red for the failure it exists to catch*.
Rev 2 attacked all four; an adversarial review of Rev 2 found that **only one was actually
closed**, and that two of Rev 2's own fixes had introduced fresh instances of the same family.
Rev 3 is the result. The score is recorded rather than smoothed over, because "I fixed it" was
wrong three times out of four and the mechanism that caught that is the one worth keeping.

| # | Rev 1 defect | Rev 2 attempt | Rev 3 |
|---|---|---|---|
| **P0** | The ONE gate is `floor < intended_delta`; the **right-hand side was never defined and would be set by the author who measures the left.** | **FAILED.** The RHS shipped as the literal string `PENDING`, and Rev 2's own `[gating]` table scheduled it to be filled **after** R0e measures the floor. A sha256 wrapped around a placeholder — the identical defect, one indirection down. | **Fixed by ordering, not hashing.** The claim fields now block **R0e** (the rung that measures the floor), not R0f (the rung that compares). The number that could be tuned to fit the floor must exist *before* the floor does. §0.1. |
| **D1** | K1's statistic is **capped at 1 by construction** — `vb_id` is one `R32G32_UINT` texel per pixel, so `distinct pairs / covered pixels ≤ 1`. | **Diagnosis right, fix broken.** Two ways: the frozen rule string `all_three_below` **inverted** the third conjunct (a `_max` among two `_min`s), so K1 did not fire on the canonical no-mechanism scene; and the decisive conjunct `submitted/covered < 1.0` is precluded by R0b's own high-poly corpus gate — self-satisfied out of existence. | **Replaced with a ladder-convergence estimator** that is uncapped, tight, and self-validating, plus two conjuncts pointing the same way. §5.5. |
| **D2** | The census resolution was **anchored to nothing**. | **WORKED** — the one fix that survived review. | Kept, plus the missing extent assertion (an OS-clamped window fabricates the curve). §5.4, §8 R0c. |
| **D3** | R0 had **no gate on its most likely branch** — §11 measures no UE5 on this box. | **Half.** The re-derived negative is a real improvement, but R0a's field-list gate was unsatisfiable on that branch, the re-derivation searched an author-written path list, and **R0f′ assumed an absolute-time instrument that §7's paired-delta harness structurally cannot be** — its algebra exists to *cancel* the absolute terms. | Branch-specific field lists; a non-authored install search; and absolute mode gets its **own, honestly worse, measured** floor. §8 R0a, §8 R0f′. |

**Two structural changes Rev 3 makes as a consequence.**

* **The frozen file is split in two.** Rev 2 put author-frozen thresholds and owner-fillable VALUES
  calls in one hashed unit, so the recorded hash was *guaranteed* to break at the first legitimate
  `PENDING` fill — and once "re-record the hash" is routine, the tripwire carries no signal and can
  launder a simultaneous threshold edit. Now: [`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml)
  is hashed and **never changes**; [`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) is
  **not hashed** and is gated by the `PENDING`-sentinel rule `goldens/PINS.toml:15` already defines.
* **The claim's scope is named.** Rev 2 compared a **per-pass** floor to a **frame-total** claim
  with no composition rule stated anywhere, which made the ONE gate not evaluable. The claim is now
  explicitly about the **bracketed VB pass chain**, and the chain floor is measured directly on the
  chain rather than composed from per-pass floors (the passes share occupancy and a queue; they are
  not independent).

**Why R0 exists.** The research's headline result is a refutation: no measured Nanite cost map
exists in any source five survey lenses could reach, so *"faster than Nanite"* is not currently
falsifiable. R0 builds the instrument that makes it falsifiable — or proves it cannot be built,
which is equally valuable and vastly cheaper than discovering it in month six.

**This document states no measured number in prose — with one fenced exception, §11.** Every fact
that could drift is a named test, and the test name is the citation. Numbers that appear are
either *structural counts* (how many files include a header; how many `.spv` a change perturbs) or
explicit `MEASURE` placeholders a rung fills in **code**, under the standing discipline:
**"MEASURED — do not edit these literals to make a failing run pass."** §11 is a dated environment
record; **no gate reads it**, and any rung that depends on one of its facts re-derives it in its
own test. That rule exists because in the sibling VB-SV0 plan hand-copied numbers in prose caused
every revision to introduce defects at the lines it edited.

**Three corrections to the R0 paragraph in the research synthesis, all verified against the tree.**

| Research says | Verified |
|---|---|
| R0 has *"no render change whatsoever … byte-identical goldens"* | **Half true.** The density census cannot read the visibility buffer without widening the `vb_id` ring's image usage — `targets.rs:868` declares `COLOR_ATTACHMENT \| SAMPLED`, no `TRANSFER_SRC`. R0c therefore makes a **device-object** change. Frame content is unaffected and the byte-identity of all VB pins is that rung's gate, but "no render change whatsoever" is withdrawn. |
| *(orchestrator prescription)* `vb_geom_fetch.hlsli` *"is included by EIGHT shaders"* | **REFUTED.** `grep -rn 'include "vb_geom_fetch'` over `crates/boyko_rhi_vulkan/shaders/` returns exactly **four**: `vb_geo.comp.hlsl:118`, `vb_resolve.comp.hlsl:85`, `vb_shade.comp.hlsl:90`, `vb_shade_split.comp.hlsl:137`. The research doc's own corrected count (four includers, **eight** sources touching the *encoding*) is the right one — §2. |
| Research §4 item 1 includes *"plus the beginnings of a bake artifact format"* | **SCOPED OUT, on the record.** Rev 1 and Rev 2 dropped it silently while stating the other two corrections explicitly. A bake format is an output of the offline builder (research ladder R4/R5) and has no consumer at R0: nothing in R0 produces clusters, a DAG or simplified LODs, so a format authored now would be authored against no data. It returns with its first producer. The research doc's stronger point — *"There is no bake stage. This is the actual first blocker and no survey named it"* — stands and is why §3 exists. |

---

## 0. What R0 is, and the three ways it kills the campaign

**R0 = a high-poly ingest path + a licence-clean corpus + a screen-space triangle-density census +
a Nanite reference capture + a decidability statement.** No meshlet, no cluster, no DAG, no shader
that did not exist before.

**The ONE gate — restated so that both of its sides can fail:**

> **`joint_floor < claim`**, both sides scoped to the **bracketed VB pass chain**
> ([`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) `[scope]`), where `joint_floor` is
> measured by R0e — combined, in `nanite_relative` mode, with the reference capture's own
> cross-session floor per `[decidability].joint_floor_rule` — and `claim` is the value the owner
> wrote into [`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) **before R0e was allowed to run**,
> together with the reproducible per-pass cost table (**named GPU, named scene, named resolution,
> named error target**) the left-hand side is measured against.

### 0.1 The P0's mechanism is ordering, and Rev 2's hash was not it

The synthesis' original wording — *"a decidability floor smaller than the delta we intend to
claim"* — is a two-sided inequality with only one side specified. **"The delta we intend to claim"
is not a measurement; it is a choice, and left unpinned it is a choice made after seeing the
floor.** An author who measures a 12% floor and then declares a 15% intended delta has closed the
gate without moving anything, and no assertion anywhere fires.

**Rev 2's answer was a committed claim file with its sha256 recorded by R0a — and it did not
work.** Every field on the right-hand side was the literal `PENDING`, and Rev 2's own `[gating]`
table scheduled the fill for R0f, *after* R0e measures the floor. What was frozen was a schema.
The named red mutation — *"raise the claim after the freeze → the hash assertion reds"* — was
undemonstrable, because the first write of a real value is a legitimate, plan-sanctioned edit that
necessarily comes with re-recording the hash, and **no test can distinguish that edit from the
cheat.** Worse, `corpus.arrangement` had to be filled to run R0b, so the hash recorded at R0a was
*guaranteed* to break before the first rung that asserted it — and once "re-record the hash" is a
routine event, any re-record can carry a simultaneous edit to a K1 threshold with every rung green.

**Rev 3's mechanism is that the claim must exist before the floor does:**

* The claim fields block **R0e — the rung that measures the floor** — not R0f, the rung that
  compares them. `r0e_blocked_by = ["claim.mode", "claim.nanite_relative_chain_delta or
  claim.absolute_chain_ms"]`. R0e's test refuses to run while the value it will be compared against
  is still `PENDING`, using the sentinel discipline `goldens/PINS.toml:15` already defines (a
  `PENDING` pin makes the checker **exit 2** rather than pass).
* This does not depend on anyone noticing an edit. The commit that fills the claim necessarily
  precedes the commit carrying R0e's MEASURED literals, and the history records it.
* **The claim file is deliberately not hashed.** Its fields are *required* to change exactly once.
  Hashing a file whose schedule requires it to change is what destroyed Rev 2's tripwire.
* **The thresholds that must never change are hashed, in their own file.**
  [`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) carries K1's instrument and rule,
  the resolution ladder, the harness denominators and the scope rules — all authored before any
  measurement is reachable. R0a records its sha256; R0c, R0d, R0e and R0f re-hash and assert it.
  Because that file has **no** legitimate reason to change, a broken hash there is unambiguous.
* **Red mutation, now demonstrable on both sides:** raise the floor above the claim → the
  inequality assertion reds; edit any threshold in the thresholds file → the hash assertion reds in
  four rungs; run R0e with the claim still `PENDING` → R0e refuses.

**What ordering costs.** R0e cannot run until the owner answers §13 Q1. That is not a schedule
defect, it is the point — and it costs little, because R0e is rung five of six and R0a–R0d are
unblocked by it. `corpus.arrangement` (Q3) blocks R0b and is the only early one.

**The three kills, each a falsifiable test rather than a worry:**

| # | Kill | Test | Disposition if it fires |
|---|---|---|---|
| **K1** | **No content, no mechanism.** The corpus never approaches ~1 triangle/pixel, so cluster LOD has no mechanism of action on our content. | R0d's census, adjudicated against `[k1]`'s **three frozen thresholds** at the frozen decision resolution — conjunctively, and with the deliberately *overstating* bracket. §5.4. | **Campaign refuted.** Not "descope" — the premise is gone. §9 clause 1. |
| **K2** | **No baseline.** The Nanite reference cannot be produced on this box. | R0a's rig probe (before any engine code) — and the negative is **re-derived by the test**, not declared. §8 R0a. | **Scope restatement**, an owner VALUES call: the goal becomes an **absolute** ms/quality target, and the ladder terminates in **R0f′**, which closes the *same* inequality. §13 Q1, §8 R0f′. |
| **K3** | **Undecidable harness.** The instrument cannot resolve the frozen claim. | R0e's decidability statement, with its null control. | Every future number is arguable. §9 clause 3 — and note this is the failure mode the sibling rung actually hit. |

**Falsification-first ordering.** K2 is the cheapest to test — it needs *zero* engine code and one
operator session — so R0a runs first. K1 needs the corpus and the instrument, so it lands third and
fourth. K3 needs the corpus (cost scales with density), so it lands fifth.

**K2 no longer terminates the ladder, and that is Rev 2's D3 fix.** In Rev 1, `achievable = false`
left R0f unrunnable and the ONE gate unclosed — on the branch §11 measured as *today's reality*. The
left-hand side of the inequality is **entirely ours**: R0e measures it with no reference at all.
Only the right-hand side's provenance changes between modes. So both branches close the same gate,
and the campaign never proceeds to R1 without a falsifiability condition.

---

## 1. Naming — decided, not open

`cluster` in this codebase means **light froxel** (`cluster_cull.hlsl`, `ClusterGrid`,
`MAX_LIGHTS_PER_CLUSTER`, the whole VB-P1e campaign). Geometry uses **`meshlet`** for the leaf and
**`geo_group`** for the DAG group. `cluster` stays with lights. This is a one-way door and it is
decided; no rung re-opens it.

---

## 2. The blast radius R0 does not touch — but must state

R0 changes no shader. It nonetheless has to state the encode blast radius, because that number
shapes every rung after it and because the ladder's R2b exists purely to pay it down.

**Verified this session (grep over `crates/boyko_rhi_vulkan/shaders/`):**

* `vb_geom_fetch.hlsli` is `#include`d by **four** sources (listed in the status block).
* `vb_pack.hlsli` — which declares `VB_ID_SENTINEL` (`:19`) — is `#include`d by **six**:
  `vb_classify_count.comp.hlsl:29`, `vb_classify_scatter.comp.hlsl:24`, `vb_geo.comp.hlsl:117`,
  `vb_resolve.comp.hlsl:84`, `vb_shade.comp.hlsl:89`, `vb_shade_split.comp.hlsl:136`.
* The **encode** side is two more sources: `vb_raster.vs.hlsl:82` exports the flat `instance_id`
  interpolant (`:63`), and `vb_raster.fs.hlsl:25` is literally
  `return uint2(input.instance_id, raw_prim_id);` with `raw_prim_id : SV_PrimitiveID` (`:24`).
* **Eight sources total touch the encoding.** They compile to **sixteen** committed `.spv`
  (`vb_raster.{vs,fs}`, `vb_geo{,_mv}`, `vb_classify_{count,scatter}`, `vb_resolve{,_froxel}`,
  `vb_shade{,_tex,_froxel,_tex_froxel}`, `vb_shade_split{,_tex,_hwrt,_tex_hwrt}`).
* **Only ten of the sixteen have a re-DXC byte-identity gate** — `vb_sv0_offpath.rs:121-130`
  enumerates exactly those ten. `vb_raster.vs`, `vb_raster.fs`, `vb_geo`, `vb_geo_mv`,
  `vb_classify_count`, `vb_classify_scatter` would drift **silently**.

**The decode side is genuinely one line** — `vb_geom_fetch.hlsli:521` is exactly
`uint local_tri = raw_prim_id % tri_count;`. The **encode** side is not independently reachable: the
G lane is filled by a fixed-function system value, so authoring a meshlet id into it requires a mesh
shader, one draw per meshlet, or a software rasterizer. **The re-encode is downstream of the
raster-path decision, not independent of it.** R0 records this and touches none of it.

---

## 3. Ingest — what exists, and what a high-poly importer must produce

### 3.1 What imports geometry today

**Exactly one mesh loader exists.** `MeshGpu::LOADERS` is a single-entry compile-time table
(`mesh.rs:237`) holding `ObjMeshLoader`, whose `EXTENSIONS` is `&["obj"]` (`loaders/obj.rs:60`). It
decodes to `MeshData { vertices: Vec<Vertex>, indices: Vec<u32> }` and runs `generate_tangents` once
over the whole mesh (`:94-96`). **There is no `.obj` file anywhere in the tree** — the loader has
never been pointed at a committed asset.

### 3.2 The contract an importer must satisfy

The importer's *only* obligation is to produce a `MeshData`. Everything downstream already works:

| Seam | Contract | Anchor |
|---|---|---|
| `Vertex` | `#[repr(C)]`, **64 B** (static-asserted), `position`@0 / `normal`@12 / `color`@24 / `uv`@40 / `tangent`@48 | `mesh.rs:81-104` |
| Index width | `Uint16` iff unique-vertex count ≤ `U16_INDEX_VERTEX_LIMIT`, else `Uint32`; the shader reads the width from `gMeshMeta[].index_width` | `mesh.rs:124`, `mesh_assets.rs:259-263` |
| Device upload | `build_mesh_gpu(ctx, &vertices, &indices, geometry_table)` | `mesh_assets.rs:238-243` |
| VB geometry slot | claimed **iff** a live table is threaded; otherwise the record carries `VB_GEOMETRY_RESERVED_SLOT` (`0`) | `mesh.rs:169`, `mesh_geometry_table.rs:66` |
| `gMeshMeta[]` row | `{index_width, vertex_count, index_count}` padded to 16 B; `tri_count = index_count / 3` | `mesh_geometry_table.rs:82-93`, `:116-118` |
| Table capacity | `MESH_GEOMETRY_TABLE_CAPACITY = 4096` slots | `geometry_bindless.rs:61` |

**The streamed path already threads the table.** `impl GpuUpload for MeshGpu` sets
`type Aux = MeshGeometryTableSlot` and calls `build_mesh_gpu(ctx, &cpu.vertices, &cpu.indices,
aux.0.as_mut())` (`gpu_upload.rs:50`, `:59`). So a **loader-decoded** mesh claims a real slot and is
VB-visible. The **host-authored** primitives do not: `register_mesh`/`cube`/`plane` pass `None`
(`mesh_assets.rs:529`) — the explicit VB sibling is `MeshAssetsVbExt::register_mesh_vb`
(`mesh_assets.rs:619-631`, `:647`), which every VB fixture uses.

> ⚠️ **A stale-documentation trap that will mislead the importer's author.**
> `const VB_IMPLEMENTED: bool = true;` (`render_path_config.rs:128`), and the table resolves as
> `path == VisibilityBuffer && mesh_leg && caps.storage_buffer_array_non_uniform_indexing`
> (`:888-890`). But **at least six doc comments still assert it is `false`** —
> `mesh_assets.rs:230` and `:290`, `runner.rs:572` and `:575`, `lib.rs:185`,
> `mesh_geometry_table.rs:400`, `geometry_bindless.rs:43`, `device.rs:704`, and
> `render_path_config.rs:25` itself. An author reading `build_mesh_gpu`'s doc concludes the
> geometry table is never armed. R0b's second red mutation (§8) exists to catch exactly the class
> of bug that belief produces.

### 3.3 The format decision — decided here, with reasons, not escalated

**Decision: glTF 2.0 binary (`.glb`), in-house decoder, deliberately narrow subset.**

* **Why not extend OBJ.** Licence-clean high-poly corpora ship as `.glb`/`.gltf`. OBJ carries no
  tangents, no index buffer (the loader sort-dedups every corner — `loaders/obj.rs:39`), and is a
  text parse over hundreds of megabytes.
* **Why in-house.** A `.glb` is a 12-byte header + a JSON chunk + a BIN chunk; only the JSON chunk
  needs a new reader. That is loader code, not hot-path code, and the same class of work
  `boyko_image`'s in-house PNG/zlib/DEFLATE already carries. §13 Q2 asks the owner only the
  **dependency-policy** half, which is a VALUES call; the format itself is decided.
* **The subset, stated as a scope cut rather than discovered as a bug.** Supported:
  `mode == TRIANGLES`, `POSITION`, `NORMAL`, `TEXCOORD_0`, `TANGENT`, `COLOR_0`, and indexed
  primitives with `u16`/`u32` indices. **Unsupported and a hard decode error, never a silent
  fallback:** sparse accessors, Draco/meshopt compression, animation, skins, morph targets,
  non-triangle modes, and non-indexed primitives. A missing `TANGENT` runs the existing
  `generate_tangents` post-pass; a missing `COLOR_0` takes `loaders/obj.rs:13`'s neutral default.
  Refusing loudly is the point: a partial mesh silently accepted is a census that measures a
  different scene than the reference capture does.

### 3.4 The residency hazard, named because nothing else names it

`build_mesh_gpu` creates **both** buffers as `MemoryLocation::HostVisibleCoherent`
(`mesh_assets.rs:295-305` for the vertex buffer; the index buffer follows). Every mesh in this
engine lives in host-visible memory, seeded once and read-only thereafter (`mesh.rs:129`). At 64 B
per vertex a multi-million-triangle corpus mesh is a large host-visible allocation, and on a
discrete GPU without resizable BAR that heap is small. **R0b's gate includes "the corpus's largest
mesh registers without allocation failure"**; the abort route is a device-local + staging upload
path for meshes, which does not exist today and is a named follow-up, not R0 work.

---

## 4. Corpus — the decision, and the constraint that forces it

### 4.1 The convention that cannot be followed

`crates/boyko_app/assets/pbr_fixtures/README.md:1-6` documents the existing convention: *"Tracked,
in-repo ground-truth oracle texture sets — small … unlike `assets/materials/`, which is
gitignored."* `.gitignore` carries the counterpart rule (`/assets/materials/*` with a
`!/assets/materials/README.md` escape). There is **no `.gitattributes`**, so **Git LFS is not
configured**, and git history is immutable — a corpus committed once is carried forever by every
clone. §11 records the measured sizes that make this decisive.

### 4.2 Three candidates, and the decision

| Candidate | Verdict |
|---|---|
| **Tracked and small** | **Rejected.** A high-poly corpus is not small by any definition that keeps this repo cloneable, and there is no LFS seam to hide it behind. |
| **Generated procedurally at test time** | **Rejected as the corpus — adopted as the instrument's self-test.** A procedural generator has a density knob, so a density census run against it can always be cranked past ~1 triangle/pixel. That makes **K1 unfalsifiable by construction** — a gate that cannot go red for the failure it exists to catch, which is this campaign's single most-repeated defect. It is however the ideal *sensitivity control* for the census instrument (§8 R0c), where an analytically-known screen-space triangle size is exactly what is wanted. |
| **Fetched, gitignored, pinned by content hash** | **CHOSEN.** |

### 4.3 The chosen shape

* A committed, human-readable manifest `assets/vg_corpus/CORPUS.toml` — per asset: source URL,
  **licence identifier and licence URL**, sha256 of the archive, sha256 of each extracted `.glb`,
  triangle count as published, and the camera-path id it is censused under. The manifest is
  **tracked**; the payload is **gitignored** by a `/assets/vg_corpus/*` + `!CORPUS.toml` +
  `!README.md` rule mirroring the `assets/materials/` precedent exactly.
* **Licence-clean means recorded, not assumed.** The repo carries no `LICENSE` file of its own, so
  the corpus manifest is the only place a licence claim can live. An asset whose licence permits
  redistribution but not the *reference capture* (e.g. loading it into a third-party engine) is
  unusable for this campaign and must be rejected at manifest-authoring time, not at R0f.
* **The same bytes feed both engines.** The Nanite reference (§6) imports the identical `.glb`
  files. If an asset cannot be imported by both, it is not corpus material.
* A `fetch_corpus` script verifies every pinned hash before extraction and refuses on mismatch. The
  **gate that reads it is a Rust test**, not the script — §8 R0b.

---

## 5. The density census — what exists, what must be added

### 5.1 Counters that exist today

* **Submitted triangles, host side.** `DrawBatch { mesh_id, index_count, index_type, base_instance,
  instance_count }` (`mesh_draw.rs:80-98`) is gathered per frame; `index_count / 3 *
  instance_count` is the submitted-triangle count with no new plumbing.
* **Per-pass GPU time, partially.** `VbTimedPass` (`gpu_timing.rs:203`) brackets **three** passes:
  `CullReset` (`:211`), `CullDispatch` (`:214`), `VbShade` (`:219`); `VB_PASS_COUNT = 3` (`:232`).
  **The VB raster pass, the `vb_geo` pass and the classify chain are NOT bracketed.** A per-pass
  table comparable to a Nanite capture therefore requires extending this enum — R0e.
* **A CPU coverage rasterizer.** `crates/boyko_app/tests/sv0_oracle/mod.rs` ships `rasterize`
  (`:279`) producing a `Coverage` (`:211`) of `CoveredPixel` (`:193`) with `covered_count`
  (`:253`), plus `changed_covered_pixels` (`:798`). It is perspective-correct and supports
  translation-only instances.

### 5.2 Counters that do not exist

Nothing anywhere produces a **screen-space triangle-size histogram** or a **triangles-per-pixel**
statistic, and nothing reads the visibility buffer back to the host. `vb_id` is created with
`usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED` (`targets.rs:868`) — **no
`TRANSFER_SRC`** — and `frame_driver.rs:750` records that the engine deliberately has *"NO
`copy_image_to_buffer(depth)`"*; the only host readback path is the swapchain
(`host_dump.rs`, `BOYKO_HOST_DUMP`).

### 5.3 The instrument — decided with structure, not escalated

| Option | Cost | Verdict |
|---|---|---|
| (a) Widen `vb_id` usage with `TRANSFER_SRC`; `copy_image_to_buffer` on census frames; histogram on the host | +1 usage bit, +1 recorded copy on armed frames only, **zero** new `.spv`, **zero** manifest rows | **CHOSEN** |
| (b) A compute pass that histograms `vb_id` into an SSBO | a new `.spv`, a new `SHADER-VARIANT-MANIFEST.md` row, a new binding, a new barrier | Rejected — buys nothing (a) does not, and enlarges the very blast radius R0 exists to keep at zero |
| (c) Reuse the CPU rasterizer alone | zero engine change | Rejected **as the census** — it is a host mirror of the raster, not the shipped VB path, and the whole point of the census is to measure what the engine actually produces. Retained as R0c's cross-check |

`copy_image_to_buffer` already exists in the RHI (`boyko_rhi/src/encoder.rs:115`; impl at
`rhi_impl/encoder.rs:1031`). The census is armed by an env knob and threaded as an `Option`, so an
unarmed frame records **zero** extra commands — the exact discipline
`Option<&VbTimestampCollector>` documents (`gpu_timing.rs:238-243`) and the reason the golden
command stream stays byte-identical.

### 5.4 The statistic — a bracket, because the obvious one is capped at 1

**Rev 1's defect, stated plainly.** `vb_id` is an `R32G32_UINT` image (`targets.rs:865`) — **one
`(instance_id, raw_prim_id)` pair per pixel**. So `distinct (instance_id, local_tri) pairs ÷
covered pixels` is **≤ 1 by construction**, saturating exactly when every covered pixel carries its
own triangle. It cannot distinguish *"we have just reached one triangle per pixel"* from *"we are
ten times past it"* — which is the entire regime the campaign exists to serve. A K1 phrased as
*"never approaches ~1"* against a statistic that **can never exceed 1** is not a threshold, it is a
ceiling being mistaken for a reading.

Per censused frame, from the readback pairs, `local_tri = raw_prim_id % tri_count` reproducing
`vb_geom_fetch.hlsli:521` on the host:

1. **`visible_tri_per_covered_pixel`** = distinct `(instance_id, local_tri)` ÷ covered pixels.
   In `(0, 1]`. **Saturating** — it *understates*, and by exactly the amount that matters most.
2. **`submitted_per_covered_pixel`** = §5.1's `index_count / 3 * instance_count` summed over
   `DrawBatch` ÷ covered pixels. **Unbounded** — it *overstates*, because submitted triangles
   include back-face-culled and off-screen ones.
3. **Screen-space triangle-size histogram** = covered pixels per distinct
   `(instance_id, local_tri)`, bucketed by powers of two, reported as a distribution — **not** a
   mean. Sub-pixel triangles never appear in it (they lose the coverage race), which is the same
   blindness as (1) and the reason (2) is carried alongside.

**Rev 2 made (2) K1's decisive conjunct, and that was wrong — the useful kind of wrong.** It is a
valid upper bound: submitted triangles are a superset of visible ones, so `submitted/covered ≥
visible/covered` always. It is also **so loose as to be inert.** It counts back-face-culled and
off-screen geometry, so it conflates *"the triangles are small"* with *"the level contains a lot of
geometry."* Firing K1 required `submitted/covered < 1.0` — the whole frame submitting fewer
triangles than the screen has covered pixels, at most ~2.07 M at 1080p — while **R0b's own gate
(b) requires each corpus mesh to match a published *high-poly* count.** A corpus that satisfies R0b
can never satisfy K1's decisive conjunct. The kill was self-satisfied out of existence, and the
demonstration is concrete: take a scene whose visible triangles are all 20+ px (a close-up of a few
large-triangle props), then place nine more copies of each asset *behind the camera*. Density is
unambiguously in the "no mechanism" regime; `submitted/covered` is ten times larger; K1 stays
silent.

### 5.5 The decisive statistic — the ladder, which costs nothing new

**The ladder frozen for D2 turns out to be the instrument D1 needed.** For a fixed camera and fixed
geometry, a triangle's screen-space area scales *exactly* with pixel count: a triangle covering 4 px
at 2160p covers 1 px at 1080p. Therefore

* `visible_tris(R)` — distinct `(instance_id, local_tri)` in the readback at resolution `R` — is
  **monotonically increasing** in `R`, because raising resolution lets smaller triangles win
  coverage races they previously lost;
* it **converges**, as `R` grows, to the true count of front-facing, unoccluded triangles in view;
* so measuring at the **top** rung reveals precisely the sub-pixel population the decision
  resolution hides — which is the population this campaign exists to serve.

The uncapped density estimate at the decision resolution is then

> **`D_est = visible_tris(top rung) ÷ covered_pixels(decision_resolution)`**

— **unbounded** above, unlike (1), and **tight**, unlike (2).

**And it validates itself, which is the part that matters.** The ladder gives four points on a
convergence curve. If the top two rungs still differ by more than
`[k1_instrument].ladder_convergence_margin = 0.05`, then `visible_tris` has **not** converged,
`D_est` is an underestimate of unknown size, and the census reports **"density not resolved"** —
**K1 is not adjudicated at all.** That is §9 clause 4's discipline (*the instrument is blind, not
the effect absent*) applied **before** the fact instead of after it. The disposition is to extend
the ladder upward in a new revision, never to adjudicate on a known underestimate.

The histogram's power-of-two shift is the same relation and is independently checkable: between
adjacent ladder rungs the modal bucket must move by exactly **two** buckets. Four rungs give three
such checks, and a curve that fails them is a curve whose scaling assumption is broken — reported,
not averaged.

**Resolution — D2's fix, kept.** The census runs `[census].resolution_ladder` and reports a
**curve**. K1 is adjudicated at `[census].decision_resolution` = 1080p **alone**, frozen: 2160p
would flatter the campaign, 512² would refute it unfairly. **512²'s real justification is narrower
than Rev 2 claimed** — it is the extent every VB fixture and golden pin already uses
(`sv0_scene/mod.rs:162`), which is what makes R0c's *procedural-fixture* cross-check possible.
It does **not** make a corpus cross-check possible: `sv0_oracle::rasterize`
(`sv0_oracle/mod.rs:279-287`) takes **one** indexed mesh and `instances: &[[f32; 3]]` — pure
translations — so it cannot rasterize a multi-asset corpus or place a rotated instance at any
resolution. R0c gate (c) is therefore scoped to the fixture, explicitly.

**The extent must be asserted, not assumed.** This engine's render extent is a real OS window
client area (`window.rs:252`, `AdjustWindowRectEx` at `:310`), and OS clamping is *already* a
recorded hazard here at 512² — `sv0_deferred_term_bench.rs:297-299` checks it, because *"an
OS-clamped window would silently measure a different per-pixel workload."* A display that clamps
1440p and 2160p produces three plausible rows and a **fabricated curve**, and every conclusion
above rests on the scaling law those rows are supposed to demonstrate. `[census]
.assert_achieved_extent` makes the readback's own dimensions the check.

**No error target is needed, and this is why.** The census renders at **full detail** — this engine
has no LOD, so there is nothing to hold an error target against. That makes the censused density
the **ceiling** of the mechanism available to any LOD scheme: a cluster hierarchy can only reduce
triangles below it. If the ceiling does not reach the regime, no LOD scheme reaches it either.
K1 is therefore decidable today, without the error target Rev 1's phrasing implied it needed.

All statistics are reported per camera path, path definitions committed as test constants — the
shape `sv0_scene/mod.rs:149-162` already uses for its camera.

---

## 6. The Nanite reference — stated plainly, including what it demands

### 6.1 What the reference must contain

UE5, **our** GPU, **our** resolution, **our** corpus, `r.Nanite.MaxPixelsPerEdge` **pinned** (its
default and its aggressive setting change rendered triangle count by roughly an order of magnitude,
so an unpinned comparison is not a comparison), per-pass milliseconds recorded with the pass names
documented. **The multi-view constraint is a fairness requirement, not a footnote:** a lit Nanite
frame runs cull+raster once per view, and any table that reports only the primary VisBuffer is
comparing one of our passes against a fraction of theirs.

### 6.2 Whether it is achievable on this box — measured, not assumed

**It is not achievable today, and the reason is concrete.** §11 records the probe: there is **no
UE5 installation on this machine** (the only Epic-shaped directory is empty), together with the
measured free space on both volumes. The operator must therefore supply, as a prerequisite to R0f:

1. a UE5 install of a named version, with disk headroom for the editor **plus** a project **plus**
   its derived-data cache — and this project's standing hazard is that the Rust `target/` directory
   alone has filled this disk to zero and masked itself as linker errors;
2. a project that imports the §4 corpus with Nanite enabled;
3. a capture protocol — `stat GPU`, Unreal Insights, or RenderDoc — producing per-pass timings, with
   the same clock-pinning discipline §7 imposes on our own harness.

**If any of the three cannot be supplied, K2 fires**, and the disposition is not "measure something
else": it is a **scope restatement** the owner makes consciously (§13 Q1). The whole falsifiability
argument for this campaign rests on this rung, which is why it runs first and why R0a's gate is
mechanical rather than a paragraph.

### 6.3 The reference's own floor — a term Rev 1 never had

A capture is an instrument too. **A claim smaller than the reference's own reproducibility is
unfalsifiable no matter how good our side is**, and Rev 1's `joint_floor` named a pair of
instruments while defining only one. So the reference capture is repeated across
`[decidability].sessions` separate sessions on the identical scene, camera and settings, and the
relative peak-to-peak spread of its per-pass medians **is** the reference floor.

The two floors combine by `[decidability].joint_floor_rule = "sum"`, and the reason is stated
rather than conventional: **quadrature assumes two independent draws from one noise process, and
these are not that.** A systematic capture bias — a different clock discipline, different pass
boundaries, a driver-side difference between the two engines — is not an independent random error,
and adding it in quadrature would understate it. Summing is conservative in the direction that
makes the campaign's own claim harder to close, which is the correct direction for a gate whose
purpose is to keep us honest.

---

## 7. The decidability statement — the harness contract

**This is not optional and it is not generic.** The sibling rung
`crates/boyko_app/tests/sv0_deferred_term_bench.rs` MEASURED, on this exact hardware, two failures
that R0's harness must be built to avoid:

* **A null control that read a third of the signal.** Strict `A,B,A,B` interleaving aliased the A/B
  phase with the frame-in-flight slot, because `FRAMES_IN_FLIGHT == 2`
  (`crates/boyko_render/src/ui/mod.rs:87`). Each phase therefore always landed on the same query
  pool, descriptor ring slot and staging region. The fix is a counterbalanced **ABBA quadruple**
  whose statistic is `(d1 + d2)/2` and whose *residual* `(d1 − d2)/2` is **printed, not hidden**
  (`sv0_deferred_term_bench.rs:53-77`).
* **A spread gate measuring its own resolution.** The timestamp counter's *step* is not the
  `timestampPeriod` the device reports; the harness had to recover it as the **GCD of raw tick
  counts** over a whole session (`:83-100`). A "cross-session spread" that is one lattice step
  carries no information.

**R0's harness MUST therefore, non-negotiably:**

1. counterbalance (ABBA), and **report** the order-bias residual with its own band;
2. carry a **null control** — two identical configurations — with a **pre-registered** maximum, as
   `SV0_NULL_CONTROL_MAX_FRACTION` (`:312`) does, fixed before the run and never widened;
3. **measure** the counter quantum by tick GCD and report it alongside `timestampPeriod`
   (`:359`, `:368`, `:377`);
4. state the **resolvable delta with confidence intervals**, and make the effective spread gate
   `max(stated gate, measured median lattice / |median|)` (`:286-295`);
5. discard warmup, run ≥3 separate processes, and pin every session's transcribed number as a test
   literal under the MEASURED discipline.

**One trap the R0e implementer will otherwise hit.** Every `read_query_pool_ns` reader requests all
of its collector's `(begin,end)` pairs with `VK_QUERY_RESULT_WAIT_BIT`, which **blocks forever** on a
pair its recorder never wrote that frame — `gpu_timing.rs:334` states this, and it is why three
separate collectors exist rather than one widened `PASS_COUNT`. Extending `VbTimedPass` to cover
raster/geo/classify means **every added pair must be written unconditionally on every armed frame**.
R0e therefore also lands a **written-pair bitmask asserted before the read**, so a conditional
bracket fails as a red assertion instead of hanging the test binary — a hang is not a gate.

---

## 8. Rungs

Ladder: **kill the baseline cheapest → land content → land the instrument → run the census → state
decidability → close the inequality** (R0f *or* R0f′, whichever branch R0a selected). Each rung is
independently committable, has **one** gate, and names the mutation that turns it red. *A mutation
that is only argued does not count; the commit message records the mutated run's output.*

### R0a — the reference-rig probe (zero engine code) — **kills K2 cheapest**

**Lands:** `docs/VG-R0-REFERENCE-RIG.toml` — a machine-readable record: UE version string, install
path, GPU name, driver version, capture tool + version, render resolution, `MaxPixelsPerEdge`, free
disk on the install volume, **the sha256 of
[`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) — the freeze (§0.1); the claim file is
deliberately *not* hashed** — the **pass-correspondence map**, and a per-pass table for **one stock
UE5 scene** (no corpus needed).
Plus `crates/boyko_app/tests/vg_r0_reference_rig.rs` reading it.

**The record has two shapes, and the gate says which fields each requires.** Rev 2 demanded *"every
field present and not `PENDING`"* over a list including the UE version string, the capture tool and
a stock-scene pass table — **none of which can exist on the `achievable = false` branch.** As
written, R0a could not pass on its own most likely outcome: the same structural hole D3 names,
relocated from the assertion into the field list.

**Gate (one) — `achievable = true` branch, four parts:** (a) every field in the *positive* set
present and not the `PENDING` sentinel — the same discipline `goldens/PINS.toml:15` defines;
(b) the recorded **GPU name matches the one this engine reports at boot** on this box — a
mechanical cross-check, not a transcription; (c) the recorded resolution equals
`[census].decision_resolution` read from the **thresholds** file, not from a constant this rung
authors; (d) the recorded `VG-CAMPAIGN-THRESHOLDS.toml` sha256 matches the file re-hashed at test
time; and the record carries the **pass-correspondence map** — the reference's pass names for its
stock scene — which `[scope].require_pass_correspondence_map_at` puts *here*, at rung one, rather
than at R0f where it would be written with both tables already in hand (§8 R0f, P1-10's fix).

**Gate (one) — `achievable = false` branch, three parts:** (a′) the *negative* field set is present
and not `PENDING` — `reason`, `search_method`, `editor_binary_name`, `probed_at`; (b′) the
re-derivation below passes; (d) as above, unchanged — the thresholds hash is asserted on both
branches.

**RED if / mutations (DEMONSTRATED):** edit the recorded GPU string by one character → (b) reds.
Blank one field of the branch's own set → (a)/(a′) reds. **Edit any threshold in
`VG-CAMPAIGN-THRESHOLDS.toml` → (d) reds** — the P0's mutation, and the one Rev 1 had no way to
express.

**The negative is re-derived, and the search space is not the author's to choose.** Rev 1 let the
record ship `achievable = false` with the test asserting "that shape", which any author satisfies by
typing `false`. Rev 2 re-walked a `probed_paths` list — better, but **still author-parameterised**:
record `["D:\\Epic Games"]` (§11 says it is empty) and the assertion is permanently true, while a
UE5 installed to `C:\Program Files\Epic Games\UE_5.4` fires nothing. So Rev 3:

* the test enumerates **fixed volumes itself** and consults the Epic launcher's own manifest
  (`C:\ProgramData\Epic\UnrealEngineLauncher\LauncherInstalled.dat`) plus the registry — a search
  space the record *describes* (`search_method`) but does not *define*;
* it asserts no `editor_binary_name` is found by that search. A UE5 installed anywhere the launcher
  knows about reds the stale `false`.

**Free disk is recorded as evidence and is deliberately NOT an assertion.** Rev 2 asserted free
space below a recorded `required_free_gb`, which fails twice over: the number is author-set (set it
to 500 GB and it can never be met), and its truth value is **controlled by the build directory** —
§11 records `target/` at 58 GB on a volume with 16 GB free, so a routine `cargo clean` flips
"below" to "above" and reds R0a with nothing broken. An assertion a housekeeping command can
falsify is not a gate on UE5 availability. The figure stays in the record, as a §11-class fact.

### R0b — corpus + ingest

**Lands:** the `.glb` decoder (§3.3) registered as a second `LoaderEntry` on `MeshGpu::LOADERS`;
`assets/vg_corpus/CORPUS.toml` + the `.gitignore` rule + `fetch_corpus`;
`crates/boyko_app/tests/vg_corpus_ingest.rs`.

**Gate (one, four parts):** (a) every corpus payload's sha256 matches its manifest pin; (b) each
`.glb` decodes to a `MeshData` whose triangle count equals the manifest's published count;
(c) each mesh, registered through the **streamed** path, lands a geometry slot
`!= VB_GEOMETRY_RESERVED_SLOT` and a `gMeshMeta` row whose `index_width` / `vertex_count` /
`index_count` match the decoded mesh; (d) the largest corpus mesh registers without allocation
failure (§3.4).

**RED if / mutations (DEMONSTRATED):**
* flip one byte of a pinned hash in `CORPUS.toml` → (a) reds;
* **register the same mesh through host-authored `register_mesh` instead of the streamed path →
  slot is `0` → (c) reds.** This is the rung's most important mutation: it is exactly the bug the
  stale `VB_IMPLEMENTED == false` comments (§3.2) will induce, and without it (c) is satisfied by
  any code path that happens to work.
* declare a `TANGENT`-less asset and delete the `generate_tangents` post-pass → (b)/(c) survive but
  the tangent lane is identity; asserted separately so the fallback cannot rot silently.

**Skip policy:** the payload is gitignored, so (a)–(d) skip when it is absent — the same shape as
the `dxc`-dependent gates (`cluster_cull_spv_sync.rs:196-204`). **Procedural mitigation, and it is
binding: the rung is not commit-eligible until the gate has been run with the corpus present and
its output pasted into the commit message.** A gate proven only on a box that skipped it is not a
gate.

### R0c — the census instrument + its sensitivity control

**Lands:** `TRANSFER_SRC` on the `vb_id` ring (`targets.rs:862-872`); an `Option`-threaded census
readback armed by env knob; the host-side histogram + triangles-per-pixel reducer;
`crates/boyko_app/tests/vg_density_census.rs`.

> ⚠️ **R0c lands the first in-frame image readback in the shipped recorder, and that is a bigger
> step than "reuse an existing seam" implies.** Every `copy_image_to_buffer` call site in this tree
> today is under `crates/boyko_rhi_vulkan/tests/`; there is **none** in `src/present/`, and
> `frame_driver.rs:750` records that the engine deliberately has no depth readback. So R0c adds
> (i) a new layout transition of a **ring** image — `COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL
> →` its `SAMPLED` read — inside the RDG auto-barrier system, and (ii) a **host read of a per-FIF
> resource**, which is the exact shape of this project's recorded cross-frame bug class (host
> access racing the fence on per-FIF rings, with `FRAMES_IN_FLIGHT == 2` at `ui/mod.rs:87`).
> Neither is visible to gate (a), because both exist only on **armed** frames — the frames the
> goldens never render. The readback must therefore wait on the frame's own fence before mapping,
> and that ordering is asserted in the rung's own test, not assumed.

**Gate (one, five parts):** (a) **every VB image golden byte-identical** to its `PINS.toml` pin
with the census unarmed — the usage widening and the unarmed `Option` must cost nothing. *Scoped to
the blessed legs:* §9 clause 5 records two `sha256_hwrt = "PENDING"` pins on which `golden.ps1`
exits 2 by design, and a gate quantified over an unblessed pin is the vacuous-selection defect
again;
(b) on a **procedurally generated** fixture whose screen-space triangle size is analytically known,
the census's modal bucket is the analytic bucket;
(c) the census's covered-pixel total agrees with `sv0_oracle::rasterize`'s `covered_count` **on that
same procedural fixture, at 512²**, within a **pre-registered** tolerance fixed before the run —
scoped to the fixture because the oracle takes one mesh and translation-only instances (§5.5) and
cannot reach the corpus at any resolution;
(d) the ladder is driven from `[census].resolution_ladder` in the **thresholds** file, whose sha256
the test re-asserts, the census produces one row per rung, **and the readback's own dimensions equal
the requested rung** (`[census].assert_achieved_extent`) — a ladder silently truncated, or silently
clamped by the OS, reds;
(e) **cross-process `vb_id` identity is MEASURED here, not assumed** — see R0d.

**RED if / mutations (DEMONSTRATED):**
* (b): subdivide the procedural fixture 4× → the modal bucket must move by **two** buckets. A
  sensitivity control that only asserts "the number changed" is the defect this campaign keeps
  finding; the required *direction and magnitude* is what makes it a gate.
* (a): record the census copy unconditionally instead of under the `Option` → the command stream
  changes on golden frames → pins move → red.
* (c): feed the reducer the CPU oracle's own coverage instead of the readback → (c) passes
  vacuously while (b) fails; the pairing is what proves (c) is not self-referential.

### R0d — the census run — **K1's evidence**

**Lands:** the census executed over the corpus at the committed camera paths, **at every rung of
the frozen resolution ladder**; results written to `docs/VG-R0-DENSITY-CENSUS.md` as the density
curve, and the **decision-bearing** numbers pinned as literals in the test under the MEASURED
discipline.

**Gate (one, three parts):** (a) the census is **reproduced across `[decidability].sessions` = 3
separate processes** under `[census].cross_run_gate` — **the sha256 of the readback itself**;
(b) `D_est`, the convergence check, the histogram and both report-only statistics are produced at
**every** ladder rung, so the resolution-dependence is on the page rather than in the choice of one
row; (c) the histogram's **two-bucket shift** holds across each adjacent rung pair (§5.5) — the
scaling law every conclusion rests on, checked three times rather than assumed once.

*The gate is that the instrument produced a reproducible number — **not** that the number is
favourable.* K1 is adjudicated in §9, deliberately, so that an unfavourable result cannot be
mistaken for a failing rung and quietly re-run until it passes.

> ⚠️ **`byte_identical` is a hypothesis this rung tests, not a property Rev 2 was entitled to
> assume.** Rev 2 justified it by *"a pipeline whose cross-process determinism the 24 existing
> golden pins already assert."* That justification is invalid: the pins hash an **8-bit shaded
> BMP** at 512² of a five-sphere fixture, and this project has **MEASURED** them blind below
> ~2⁻¹⁶ relative. Two adjacent triangles of a smooth mesh can shade identically to 8 bits and
> carry **different `vb_id`**. `vb_id` identity is a strictly finer function of the same state,
> and it is being asserted at 2160p on a multi-million-triangle corpus where near-coplanar
> sub-pixel triangles make coverage ties common — a regime the pins have never visited. **R0c
> measures it first** (gate (e)) and reports the result; R0d relies on it only if it held.

**If the readback proves non-deterministic** — e.g. a driver-side raster order that changes which
triangle wins a coverage tie — that is a **real finding about the raster path**, and it is recorded
as one. `[census].cross_run_spread_fallback` is adopted only *after* such a finding is entered by
name and date as a plan amendment (§11). It is not a bound anyone may reach for to make a run pass.

**RED if / mutations.** Rev 2's named mutation — *"point two of the three runs at different camera
paths"* — is **not a gate test**: it changes the test's *input*, and would red any hash of anything.
The mutations that actually probe (a) are ones that leave the shaded golden **identical** while
changing `vb_id`:
* **permute the spawn order of two identical instances** → every `instance_id` changes, the shaded
  pin is byte-identical, and (a) must red. This is the mutation that proves the gate reads `vb_id`
  and not the image;
* drop the ladder to its decision row only → (b) reds;
* scale the fixture so triangles cross a bucket boundary at one rung only → (c) reds.

### R0e — the decidability statement — **K3's test**

**Lands:** `VbTimedPass` extended to bracket the VB raster, `vb_geo` and the classify chain, with
the written-pair bitmask of §7; a counterbalanced ABBA harness over the corpus scene with a null
control; `crates/boyko_app/tests/vg_r0_decidability.rs`, all session numbers transcribed as
literals.

**Blocked until the claim exists.** R0e's test asserts `claim.mode` and its mode's delta field are
**not `PENDING`** before it measures anything, and fails if they are. That ordering *is* the P0's
fix (§0.1): the number the floor will be compared against must predate the floor.

**Gate (one, three parts), every fraction AND ITS DENOMINATOR read from
[`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) rather than minted here:**
(a) the null control's `|median paired delta|` is at or below
`[decidability].null_control_max_fraction` of **the armed paired delta**;
(b) the reported `median_lattice_ns / |median|` is at or below that fraction for **every** bracketed
pass — a pass whose cost sits at the lattice is reported as **not resolvable**, by name, rather than
averaged into a total;
(c) the cross-session spread of the **paired deltas** over `[decidability].sessions` processes is
within `max([decidability].session_spread_max, measured lattice / |median|)`;
(d) the chain total's floor is measured on **one bracket spanning the chain**
(`[scope].chain_floor_rule`), not composed from the per-pass floors — the passes share occupancy,
caches and a queue, so arithmetic composition would assume an independence they do not have.

> ⚠️ **The denominators are the gate.** Rev 2 transferred the sibling's three literals while
> silently changing what they divide. The sibling gates the null control against the **armed
> delta** — it fired at 33% (−2048 ns against a 6144 ns signal) and *that failure is what produced
> the ABBA redesign*. Rev 2 gated it against *"the smallest per-pass median"*, an **absolute** cost:
> a pass costing 100 µs would carry a 10 µs null budget where the sibling's was 614 ns, roughly a
> 20× weakening, **and the precedent's own red event would have passed.** R0e's first named
> mutation — revert to strict ABAB — would then not have fired. Same for (c): this project has
> recorded ~21% run-to-run spread on **absolute** GPU pass costs at high N (the VB-P1d bench), so a
> 0.10 gate on absolute medians would red for a known instrument property rather than a finding.
> Both denominators are now written down (`null_control_denominator`, `session_spread_denominator`).

The three literals themselves are not invented for this campaign — they are the ones
`sv0_deferred_term_bench.rs:350`, `:366`, `:378` already carry, **measured on this exact box**, by
a rung that had never heard of virtual geometry.

**RED if / mutations (DEMONSTRATED):**
* revert the harness to strict ABAB → (a) must exceed its budget. This was **measured** on this
  hardware in the sibling rung, so it is a re-demonstration, not a hope;
* make one added bracket conditional on a branch the fixture never takes → the written-pair
  bitmask assertion reds (instead of the `WAIT_BIT` readback hanging);
* halve the sample count → the confidence interval widens past the pre-registered bound → red.

### R0f — the reference capture — **closes the ONE gate** (`nanite_relative` mode)

**Runs only if R0a recorded `achievable = true`.** Otherwise the ladder terminates in R0f′ below.

**Lands:** the corpus imported into the R0a project; per-pass Nanite timings at the pinned error
target and the pinned resolution, for the **same camera paths**, **repeated across
`[decidability].sessions` capture sessions** (§6.3), recorded into
`docs/VG-R0-REFERENCE-RIG.toml`'s table; and the campaign's **decidability statement**: the smallest
delta this pair of instruments can jointly resolve.

**Gate (one, four parts):** (a) a reproducible per-pass table with named GPU / scene / resolution /
error target; (b) the reference floor is derived from the cross-session spread of that table, and
`joint_floor = our_floor + reference_floor` per `[decidability].joint_floor_rule`; (c) the ONE gate
itself — `joint_floor < claim.nanite_relative_chain_delta`, **both sides scoped to the bracketed VB
pass chain** (`[scope].claim_scope`), with the claim read from a file that R0e already proved was
filled before the floor was measured; (d) the **pass-correspondence map** recorded at R0a is
**total** over our bracketed pass set — without it `nanite_relative_per_pass_regression_max` cannot
be evaluated at all, and whoever writes the map after both tables exist writes it with the answer in
hand.

**Scope, stated because Rev 2 got it wrong.** Rev 2 compared a per-pass floor against a field named
`frame_total_delta` with **no composition rule anywhere**, which made the ONE gate not evaluable.
R0e measures the VB pass chain: raster, `vb_geo`, classify, and the three passes `VbTimedPass`
already brackets. A *frame* additionally contains CSM, SDF, DDGI, post/AA, present and all CPU
time — none of which this campaign touches and none of which the harness measures. The claim is
about the chain, and the field is now named `nanite_relative_chain_delta`.

**RED if / mutations — one per side, which is the point:**
* raise the recorded floor above the claim → the inequality assertion reds (Rev 1 had this one);
* **fill the claim to fit the floor → impossible without reding R0e**, which refuses to run while
  the field is `PENDING` and whose MEASURED literals are committed after the fill. This is the
  cheaper cheat, and it is the one Rev 1 and Rev 2 both left open.

That pair is the whole campaign's falsifiability condition, and both halves must be able to fail.

### R0f′ — the absolute-mode closure — **closes the ONE gate when K2 has fired**

**Runs iff R0a recorded `achievable = false` and its negative was re-derived.** This rung exists
because Rev 1 left the *most likely* branch with no closure at all: no reference, therefore no R0f,
therefore no falsifiability condition, therefore a campaign that proceeds to R1 on an argument.

> ⚠️ **Rev 2 wrote *"nothing new to measure — the left-hand side is already ours"*, and that was
> its sharpest error.** The harness §7 mandates measures **paired differences**, and its entire
> algebra exists to *cancel* the absolute terms: in `m_k = μ + τ·armed(k) + γ(fi(k)) + β·k + ε_k`,
> the ABBA statistic `(d₁+d₂)/2` recovers `τ` **precisely by eliminating** `μ` (the per-frame
> baseline), `γ` (the frame-in-flight-slot offset) and `β` (position drift)
> — `sv0_deferred_term_bench.rs:34`, `:58-62`. Those are exactly the terms an absolute millisecond
> reading must **retain**, and this repo has measured them to be large (the null control read
> −2048 ns against a 6144 ns signal — a third of the "signal"). **A floor produced by a design that
> cancels the absolute terms says nothing about whether an absolute reading is trustworthy.**
> R0f′ therefore needs its own instrument, and it does not get to pretend otherwise.

**Lands:** an **absolute-time** measurement — per-pass and chain-total *medians*, not deltas —
across `[decidability].sessions` separate processes, with the cross-session spread of those
absolute medians reported; plus the inequality assertion in `vg_r0_reference_rig.rs` against
`claim.absolute_chain_ms`, and the corpus-and-quality context that makes an absolute target
meaningful (`[corpus].arrangement`, `[quality].arbiter`).

**Gate (one, three parts):** (a) the measured absolute cross-session spread is at or below
`[absolute_mode].absolute_session_spread_ceiling` — **derived by measurement here, not adopted**;
the ceiling is pre-registered at 0.25 from this project's recorded ~21% absolute-cost spread at
high N, with margin, and R0f′ reds if the measurement exceeds it; (b) `absolute_floor <
claim.absolute_chain_ms`, both in milliseconds on the bracketed chain at the decision resolution —
a genuine inequality between two quantities of the same kind, unlike Rev 2's dimensionless fraction
against "the resolvable fraction of a millisecond target"; (c) the thresholds file's sha256
re-asserts, and `claim.mode == "absolute"` is consistent with R0a's `achievable = false` — a mode
set to `nanite_relative` while the rig says unachievable reds, so the two documents cannot disagree
silently.

**Absolute mode is honestly weaker, and saying so is the deliverable.** Its floor is roughly
2.5× the paired-delta floor, because absolute readings keep every term ABBA was built to remove.
A weaker instrument reported plainly beats a strong-looking number the harness cannot support —
and if (a) reds, the correct reading is that **this box cannot support an absolute claim at all**,
which is a finding, not a failure.

**RED if / mutations:** set `mode = "nanite_relative"` while `achievable = false` → (c) reds.
Set `absolute_chain_ms` below the measured floor → (b) reds, and correctly: a target we cannot
measure ourselves hitting is not a target. Reuse R0e's paired-delta floor as `absolute_floor`
instead of measuring absolutes → (a) has no measurement to gate and the rung cannot report, which
is the failure Rev 2 would have shipped silently.

---

## 9. ABORT criteria

The rung is **reverted or the campaign re-scoped** — not softened mid-flight — if any of:

1. **K1 — no content.** At `[census].decision_resolution`, **both** of `[k1]`'s frozen conjuncts
   hold: `D_est < 1.0` **and** the modal bucket is **above** 16 pixels. Both point the same way —
   *large triangles* — which Rev 2's three-conjunct rule did not: its `rule = "all_three_below"`
   put a `_max` among two `_min`s, so on the canonical no-mechanism scene (a few giant flat quads)
   two conjuncts held, the third did not, and **K1 failed to fire on the exact scene it was written
   to catch.** An implementer coding from the frozen TOML would have written `modal < 16` and
   produced a kill that fires when triangles are *small* — i.e. when the campaign's premise is
   *confirmed*. The rule is now spelled out as a sentence for that reason.
   **K1 is not adjudicated at all** if `[k1_instrument]`'s ladder-convergence check fails: `D_est`
   is then an underestimate of unknown size, and clause 4 governs. Then cluster LOD has no
   mechanism of action on
   this content — at **full detail**, i.e. at the ceiling any LOD scheme could ever see — and **the
   campaign is refuted as stated**. The disposition is the owner's: change the target content class
   (and re-run R0b–R0d against it), or stop. It is explicitly *not* "generate a denser corpus" —
   §4.2 records why that makes the kill vacuous — and it is explicitly *not* "adjudicate at 2160p
   instead", which §5.4 forecloses by freezing the decision resolution.
2. **K2 — no baseline.** R0a records `achievable = false` **and the test re-derives it** (§8 R0a).
   Then *"faster than Nanite"* is unfalsifiable and the goal is restated as an **absolute**
   ms-at-quality target. **Owner VALUES call** (§13 Q1) — taken consciously, at rung one.
   **This is a re-scope, not an abort:** the ladder continues to **R0f′**, which closes the same
   inequality with `claim.absolute_chain_ms`. Rev 1 treated this branch as terminal, which left the
   campaign's *most likely* path with no falsifiability condition at all.
3. **K3 — undecidable harness.** Two distinct outcomes, which Rev 2 conflated under one clause:
   * **(3a) the instrument misbehaves** — R0e's gate reds (null control over budget, a pass sitting
     at the lattice, cross-session spread out of band). The ladder does not proceed to R1 until the
     instrument is fixed. Nothing is learned about the campaign either way.
   * **(3b) the instrument works and the answer is no** — R0f/R0f′'s inequality reds: the floor is
     real, measured, and **larger than the claim**. This is research §5's K3 as actually worded
     (*"if the resolvable delta exceeds the delta we intend to claim, no result from this campaign
     is defensible"*), and its disposition is different: the instrument is not broken, so fixing it
     is not the move. The owner either lowers the claim to something this pair of instruments can
     resolve — which may make the campaign not worth running — or invests in a better instrument.
     **Owner VALUES call**, and it is the outcome the whole R0 rung exists to surface early.
4. **The instrument is untrustworthy rather than the result being bad — and this has its own
   disposition, because it is the case that gets misread.** If R0c's sensitivity control (b) fails
   while (a) and (c) pass, or R0e's null control fails while the armed medians look tidy, the
   correct reading is *the instrument is blind*, **not** *the effect is absent*. Outcome: the rung
   is **not** commit-eligible, no number from it enters any later gate, and the failure is recorded
   in this document's §11 with its date. The sibling rung's ABAB null control is precisely this
   case: three armed sessions looked tidy and inside their gate while the control said a third of
   the "signal" was ordering bias.
5. **Golden-bless throughput.** Two of the twenty-four pins in `goldens/PINS.toml` still carry
   `sha256_hwrt = "PENDING"` (`:364`, `:409`) — their software legs are blessed, their hwrt legs are
   not. R0 moves no pin, so it is unaffected; but the first byte-moving rung of this campaign
   starts on an incompletely-green corpus, and §13 Q4 puts the bless-bandwidth question to the
   owner before that rung is scheduled, not after.

---

## 10. Risks

| # | Risk | Precedent | Mitigation |
|---|---|---|---|
| R1 | **Vacuously-green gate** — an assertion quantified over an empty or self-referential selection. | The campaign's #1 recurring defect; found five times in the sibling plan alone. | Every rung names a mutation and the commit records its output; R0c(b)/(c) are deliberately paired so neither can pass alone. |
| R2 | **A procedural corpus makes K1 untestable.** | New, and it is why §4.2 rejects the cheapest corpus option. | The corpus is fetched real content; procedural geometry is confined to R0c's sensitivity control. |
| R3 | **The harness measures its own resolution, or its A/B rides the ring.** | MEASURED in the sibling rung, both of them: a "spread" that was one median lattice step, and an ABAB phase perfectly aliased with `FRAMES_IN_FLIGHT == 2`. | §7 clauses 1, 3–4: ABBA with the residual reported; the quantum measured by tick GCD and the spread gate read against it. |
| R4 | **`WAIT_BIT` readback hangs instead of failing.** | `gpu_timing.rs:334` documents the deadlock; three separate collectors exist because of it. | R0e's written-pair bitmask, asserted before the read. |
| R5 | **Stale doc sends the importer down the `None` path.** | Verified: ≥6 comments still claim `VB_IMPLEMENTED == false` while `render_path_config.rs:128` says `true`. | R0b's second red mutation targets exactly this; fixing the comments is a separate one-line commit, deliberately not absorbed here. |
| R6 | **Host-visible residency ceiling.** | `mesh_assets.rs:295-305`: every mesh buffer is `HostVisibleCoherent`. | R0b gate (d); the device-local + staging path is a named follow-up, not R0 work. |
| R7 | **The `vb_id` usage widening perturbs a golden.** | New. | R0c gate (a) over every VB pin, with a demonstrated red (record the copy unconditionally). |
| R8 | **UE5 capture measures a different scene than our census.** | New — the two engines must load the same bytes. | §4.3: an asset that cannot be imported by both is not corpus material; R0a(c) pins the resolution across both. |
| R9 | **Disk exhaustion masquerading as a build failure.** | This project's record: `target/` has filled this disk and surfaced as linker errors. | §11 records the measured headroom; R0a's record carries free-disk as a required field, and R0a's negative branch **re-reads** it at test time. |
| R10 | **The claim is set to meet the floor.** The cheapest way to close the ONE gate is to write the number on the right after seeing the left. | The P0 of Rev 1 — **and of Rev 2, which answered it with a hash around the string `PENDING`.** | §0.1: the claim blocks **R0e**, the rung that measures the floor. Ordering, not hashing — it does not depend on anyone noticing an edit. |
| R11 | **A statistic that cannot exceed its own threshold.** `visible_tri_per_covered_pixel ≤ 1` by construction, and K1 was phrased against ~1. | Found in Rev 1 by inspection; **Rev 2's replacement was inert for a different reason** — `submitted/covered < 1.0` is precluded by R0b's own high-poly corpus gate. | §5.5's ladder-convergence estimator: uncapped, tight, and self-validating, with a *not adjudicated* branch when the ladder has not converged. |
| R12 | **The census resolution silently decides K1.** Density scales as 1/resolution². | New in Rev 2 and the one fix that survived review. | Frozen ladder + frozen decision resolution; the curve is reported at every rung; **and the achieved extent is asserted**, because OS clamping is already a recorded hazard here at 512². |
| R13 | **The most likely branch has no gate.** §11 measures no UE5 on this box. | New. | R0a's negative is re-derived over a **non-authored** search space (fixed volumes + the launcher manifest); R0f′ closes the inequality with its own absolute instrument. |
| R14 | **A frozen file whose schedule requires it to change.** A tripwire that fires routinely carries no signal, and a routine re-record can launder a threshold edit. | **Measured in Rev 2 by inspection:** its recorded hash was *guaranteed* to break at the `corpus.arrangement` fill, before the first rung that asserted it. | The split: thresholds hashed and never edited; claim unhashed and gated by the `PENDING` sentinel. |
| R15 | **A harness asked for a quantity its algebra removes.** | **Measured:** ABBA recovers `τ` by cancelling `μ`, `γ` and `β` — exactly what an absolute reading needs. Rev 2's R0f′ assumed otherwise. | `[absolute_mode]`: its own instrument, its own pre-registered ceiling, and the honest statement that absolute mode is ~2.5× weaker. |
| R16 | **A literal transferred without its denominator.** | **Measured in Rev 2:** the sibling's 0.10 null-control gate moved from *armed delta* to *absolute pass median*, a ~20× weakening under which the precedent's own red event would have passed. | Denominators written down next to every fraction in `[decidability]`. |

---

## 11. Environment record — dated, and NOTHING READS THESE NUMBERS

Fenced exception to this document's no-measured-numbers-in-prose rule. These are facts about the
machine and the tree as of authoring; they are **evidence for design decisions, not gate
thresholds**. No test reads them, and any rung that depends on one re-derives it in its own code.

**Probed 2026-07-26, this box, working tree on branch `feat/multi-paradigm-render` at `a139799`.**

* **UE5:** no installation present. The only Epic-shaped directory on either volume,
  `D:\Epic Games`, exists and is **empty** (0 entries). No `UnrealEditor.exe` anywhere probed.
* **Free space:** `C:` 71.9 GB free of 238.3 GB; `D:` (the repo volume) 18.5 GB free of 237.7 GB.

**Re-probed 2026-07-27 at Rev 2, same box, working tree at `13f1c9a`** — recorded because the
figure **moved in the direction that matters**, and because R0a's negative branch now re-derives it
rather than trusting this record:

* **Free space:** `C:` **63 GB** free of 239 GB; `D:` (the repo volume) **16 GB** free of 238 GB.
  Both fell over one day of ordinary work. `target/` alone is **58 GB** — larger than the free space
  on the volume that holds it, and this project's standing hazard is that exhausting it surfaces as
  mingw linker errors rather than as a disk error.
* **What this does to K2.** A UE5 editor install plus a project plus its derived-data cache does not
  fit on `D:` today and is uncomfortable on `C:`. K2 firing is not a hypothetical branch of this
  plan — on the measured state of this machine it is the **expected** one, which is exactly why
  Rev 2 refuses to leave it ungated (§8 R0a, §8 R0f′, §9 clause 2).
* **Repo size:** `.git` 24.6 MB; all tracked assets under `crates/boyko_app/assets/` total 1.07 MB.
  No `.gitattributes` — **Git LFS is not configured**. No `LICENSE` file at the repo root.
* **Content today:** the VB fixtures render five instances of one `uv_sphere(radius, 28 stacks,
  40 slices)` at 512×512 (`sv0_scene/mod.rs:56-69`, `:162`). Twenty-four golden pins exist; two
  carry `sha256_hwrt = "PENDING"`.
* **Shaders:** 16 committed VB `.spv` are perturbed by a `vb_id` re-encode; 10 have a re-DXC gate.

### 11.1 Amendment record

Frozen values in [`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) change **only** by a
dated entry here, in a new plan revision, never by an in-place edit. The recorded sha256 in
`VG-R0-REFERENCE-RIG.toml` is updated in the same commit, deliberately and visibly.

| Date | Revision | Value | From → To | Why |
|---|---|---|---|---|
| — | — | — | — | *No amendment yet. The thresholds file is at its authoring state.* |

**Findings that pre-authorize a fallback** (per `[census].cross_run_spread_fallback` and
`[k1_instrument].on_not_converged`) are also entered here, by name and date, **before** the fallback
is used. A fallback adopted without an entry is the "widen the gate to make the run pass" move that
§7 clause 2 forbids.

---

## 12. Appendix — verified file:line anchors

Every line below was opened or grepped while writing this revision.

**Ingest / mesh:** `crates/boyko_render/src/loaders/obj.rs:13` (default vertex colour), `:55`
(`ObjMeshLoader`), `:60` (`EXTENSIONS = &["obj"]`), `:94-96` (dedup + `generate_tangents`) ·
`crates/boyko_render/src/mesh.rs:81-100` (`Vertex`), `:103-104` (`VERTEX_STRIDE == 64`, static
assert), `:124` (`U16_INDEX_VERTEX_LIMIT`), `:137-186` (`MeshGpu`), `:169` (`geometry_slot`),
`:193` (`type Cpu = MeshData`), `:237` (single `LoaderEntry`) ·
`crates/boyko_render/src/mesh_assets.rs:238-243` (`build_mesh_gpu` signature), `:259-263` (index
width), `:290` (**stale** `VB_IMPLEMENTED == false` comment), `:295-305`
(`MemoryLocation::HostVisibleCoherent`), `:529` (`register_mesh` passes `None`), `:619-631`
(`MeshAssetsVbExt`), `:647` (`register_mesh_vb` impl) ·
`crates/boyko_render/src/gpu_upload.rs:41-61` (`GpuUpload for MeshGpu`; `type Aux =
MeshGeometryTableSlot` at `:50`; **the threaded call at `:59`**).

**Geometry table:** `crates/boyko_render/src/mesh_geometry_table.rs:17-27` (module doc),
`:66` (`VB_GEOMETRY_RESERVED_SLOT`), `:82-93` (`MeshGeometryMeta`), `:97` (16 B stride),
`:116-118` (`tri_count`), `:140-142` (`mesh_buffer_usage`), `:400` (**stale** comment), `:413`
(`MeshGeometryTableSlot`) · `crates/boyko_rhi_vulkan/src/geometry_bindless.rs:61`
(`MESH_GEOMETRY_TABLE_CAPACITY = 4096`), `:43` (**stale** comment).

**Path resolution:** `crates/boyko_render/src/render_path_config.rs:25` (**stale** module-doc
sentence), **`:128` (`const VB_IMPLEMENTED: bool = true;`)**, `:517` (`vb_geometry_table` field),
`:888-890` (the predicate).

**Encode / decode:** `crates/boyko_rhi_vulkan/shaders/vb_geom_fetch.hlsli:516`
(`vb_geom_fetch` signature), **`:521` (`uint local_tri = raw_prim_id % tri_count;`)** ·
`vb_pack.hlsli:19` (`VB_ID_SENTINEL`) · `vb_raster.vs.hlsl:63` (flat `IID` interpolant), `:82`
(the export) · **`vb_raster.fs.hlsl:24-25` (`uint2(input.instance_id, raw_prim_id)`)** ·
includers: `vb_geo.comp.hlsl:117`/`:118`, `vb_resolve.comp.hlsl:84`/`:85`,
`vb_shade.comp.hlsl:89`/`:90`, `vb_shade_split.comp.hlsl:136`/`:137`,
`vb_classify_count.comp.hlsl:29`, `vb_classify_scatter.comp.hlsl:24` ·
`crates/boyko_rhi_vulkan/tests/vb_sv0_offpath.rs:121-130` (the ten gated rows).

**Targets / readback:** `crates/boyko_rhi_vulkan/src/present/targets.rs:851-856` (`VbTargets`),
**`:868` (`COLOR_ATTACHMENT | SAMPLED` — no `TRANSFER_SRC`)** ·
`crates/boyko_rhi/src/encoder.rs:115` (`copy_image_to_buffer`) ·
`crates/boyko_rhi_vulkan/src/rhi_impl/encoder.rs:1031` (impl) ·
`crates/boyko_rhi_vulkan/src/present/frame_driver.rs:750` (no depth readback) ·
`crates/boyko_app/src/host_dump.rs:1-10`, `:67` (`BOYKO_HOST_DUMP`).

**Timing — RE-VERIFIED at Rev 3; Rev 1 and Rev 2 both carried a consistent ~10-line drift here,
i.e. anchors read from a pre-VB-P1e-H0 tree.** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs`:
`:188-194` (why collectors are separate — and the `PASS_COUNT` note), `:229` (**`VbShade = 2`**,
not `:219`), **`:242` (`VB_PASS_COUNT: u32 = 3`, not `:232`)**, `:281`/`:293-294` (the pool reset),
**`:344` (`WAIT_BIT` BLOCKS FOREVER on a pair its recorder never wrote, not `:334`)** — this one is
cited by §7's non-negotiable implementer trap and by risk R4, so the stale anchor was the most
expensive of the set — `:357` (`Sv0TimedPass`, not `:347`), **`:381` (`SV0_PASS_COUNT = 1`, not
`:371`)**.

**Harness precedent:** `crates/boyko_app/tests/sv0_deferred_term_bench.rs:20-51` (ABAB refuted by
its own null control), `:34` and `:58-62` (**the ABBA algebra — the model `m_k = μ + τ·armed + γ(fi)
+ β·k + ε` and the cancellation that makes absolute readings unavailable**, §8 R0f′), `:83-129`
(the quantisation finding), `:297-299` (**the OS-clamped-extent check**, §5.4), **`:350`
(`SV0_BENCH_SESSIONS = 3`), `:366` (`SV0_SESSION_SPREAD_MAX = 0.10`), `:378`
(`SV0_NULL_CONTROL_MAX_FRACTION = 0.10`)** — Rev 2 cited `:284` and `:312` for two of these in one
block and `:350`/`:378` in another; **the `350`/`366`/`378` set is the correct one**, and the
contradiction is direct evidence that the older block was never re-verified ·
`crates/boyko_render/src/ui/mod.rs:87` (`FRAMES_IN_FLIGHT = 2`) ·
`crates/boyko_render/src/mesh_draw.rs:80-98` (`DrawBatch`) ·
`crates/boyko_rhi_vulkan/src/window.rs:252` (`Window::open`), `:310` (`AdjustWindowRectEx`),
`:342-352` (`BOYKO_WIN_HIDDEN` — hidden, but still created at the requested size).

> **§12's opening sentence — *"Every line below was opened or grepped while writing this
> revision"* — was FALSE in Rev 2**, systematically, across the whole Timing block. It is the
> claim this project's own standing lesson exists against (*report line numbers are lower bounds;
> grep the pattern*). Every anchor in this section was re-derived at Rev 3 by grep; the ones that
> moved are called out inline above rather than silently corrected, because a silent correction
> would leave no evidence that the blanket claim had been wrong.

**Oracles / fixtures:** `crates/boyko_app/tests/sv0_oracle/mod.rs:182-208` (`OracleVertex`,
`CoveredPixel`), `:211-256` (`Coverage`, `covered_count` at `:253`), `:279-287` (`rasterize`),
`:765-798` (`ChangedPixels`, `changed_covered_pixels`) · `crates/boyko_app/tests/sv0_scene/mod.rs:56-69`
(mesh row constants), `:149-162` (camera + `DUMP_EXTENT`), `:223` (`uv_sphere`) ·
`crates/boyko_app/tests/sv0_adequacy.rs:231-232`, `:514-515` (the shared-spawn inseparability test).

**Rev 2/Rev 3 additions, verified this session:**
`crates/boyko_rhi_vulkan/src/present/targets.rs:851-856` (`VbTargets` doc — the ring is **one
`R32G32_UINT` texel per pixel**, which is what caps §5.4's statistic (1) at 1), **`:866`
(`format: Format::R32G32Uint` — Rev 2 cited `:865`, which is `depth: 1`)**, `:868` (the usage bits,
correct) · `crates/boyko_app/tests/sv0_scene/mod.rs:162` (`DUMP_EXTENT = 512`) ·
`crates/boyko_app/tests/sv0_oracle/mod.rs:279-287` (**`rasterize` takes ONE indexed mesh and
`instances: &[[f32; 3]]` — translation-only**, which is why R0c gate (c) is scoped to the procedural
fixture and cannot reach the corpus at any ladder rung) ·
`crates/boyko_render/src/mesh_draw.rs:80-98` (`DrawBatch` — the source of the report-only
`submitted_per_covered_pixel`) · `crates/boyko_rhi_vulkan/shaders/vb_pack.hlsli:15-16`, `:19`
(`VB_ID_SENTINEL` marks a pixel the mesh raster leg never covered — the census's denominator is
mesh-covered pixels, not all pixels) ·
[`docs/VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) (hashed, never edited) ·
[`docs/VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) (unhashed, `PENDING`-gated, blocks R0e).

**Corpus convention:** `crates/boyko_app/assets/pbr_fixtures/README.md:1-6` ·
`.gitignore` (`/assets/materials/*` + the `!README.md` escape) ·
`goldens/PINS.toml:15` (the `PENDING` sentinel rule), `:363-364`, `:408-409` (the two unblessed
hwrt legs) · `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:196-204` (the skip shape).

---

## 13. Open questions — VALUES / SCOPE only

Performance and architecture forks are decided with numbers in this project; the format choice
(§3.3), the census instrument (§5.3), the corpus shape (§4.2), the census resolution ladder and
K1's thresholds (§5.4) are decided above and are not listed here.

**Every question below has a field waiting for it in
[`docs/VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml)**, and that file's `[gating]` table states
which rung each one blocks. The short version: **Q3 blocks R0b. Q1 blocks R0e — deliberately, and
that ordering is the P0's whole fix (§0.1).** Q5 blocks only the final rung; nothing blocks R0a,
R0c or R0d.

**Q1 moved earlier between Rev 2 and Rev 3, and it is the one schedule cost this revision
knowingly accepts.** Rev 2 let the claim be written at R0f — after the floor was measured — which
is exactly the defect the claim file was created to fix. Answering Q1 before R0e runs is what makes
the inequality falsifiable, so the question is now on the critical path at rung five rather than
rung six. Everything up to and including the census is unblocked by it.

1. **If K2 fires, what replaces the goal?** *"Faster than Nanite"* becomes *"N ms at quality Q on
   corpus C"* — the owner sets N, Q and C. This is the single most consequential question in the
   document and it must be answered at rung one, not month six.
2. **Third-party dependency policy for the importer.** §3.3 decides *glTF, in-house*. If the owner
   will accept a third-party glTF/JSON crate, the decoder shrinks substantially — but the
   workspace's demonstrated posture is fully in-house (raw-FFI Vulkan, in-house PNG/zlib/DEFLATE).
   The same question recurs, far more sharply, for the offline builder at R4/R5.
3. **Corpus provenance and licence.** Who selects and licenses the high-poly assets, and is a
   fetched-and-gitignored payload with pinned hashes acceptable as the permanent arrangement?
   Without an answer, R0b cannot author `CORPUS.toml`.
4. **Bless bandwidth.** How many byte-moving rungs per week can the owner actually bless? R0 moves
   no pin, but two hwrt legs are already `PENDING` (§9 clause 5), and that number caps the width of
   every rung after R2b.
5. **Quality target.** What pixel-error budget counts as "equal quality" — our equivalent of a
   pinned `MaxPixelsPerEdge` — and is the owner the arbiter by visual eval, or do we bind to a
   metric? Note the standing lesson that image statistics have already misled this project twice,
   which argues against a metric.
