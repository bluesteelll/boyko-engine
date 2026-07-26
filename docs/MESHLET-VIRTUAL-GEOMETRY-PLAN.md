# VG-R0 — "The Ruler": the measurement rung of the virtual-geometry campaign

**Status:** DESIGN, **Rev 1** — **NOT APPROVED, and no code exists.** This document specifies
**only rung R0** of the ladder in [`docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`](MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md)
§4. R1–R8 stay as that document leaves them and are out of scope here. The owner's decision to
build a meshlet / virtual-geometry system is **settled** and is not re-litigated below.

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

**Two corrections to the R0 paragraph in the research synthesis, both verified this session.**

| Research says | Verified |
|---|---|
| R0 has *"no render change whatsoever … byte-identical goldens"* | **Half true.** The density census cannot read the visibility buffer without widening the `vb_id` ring's image usage — `targets.rs:868` declares `COLOR_ATTACHMENT \| SAMPLED`, no `TRANSFER_SRC`. R0c therefore makes a **device-object** change. Frame content is unaffected and the byte-identity of all VB pins is that rung's gate, but "no render change whatsoever" is withdrawn. |
| *(orchestrator prescription)* `vb_geom_fetch.hlsli` *"is included by EIGHT shaders"* | **REFUTED.** `grep -rn 'include "vb_geom_fetch'` over `crates/boyko_rhi_vulkan/shaders/` returns exactly **four**: `vb_geo.comp.hlsl:118`, `vb_resolve.comp.hlsl:85`, `vb_shade.comp.hlsl:90`, `vb_shade_split.comp.hlsl:137`. The research doc's own corrected count (four includers, **eight** sources touching the *encoding*) is the right one — §2. |

---

## 0. What R0 is, and the three ways it kills the campaign

**R0 = a high-poly ingest path + a licence-clean corpus + a screen-space triangle-density census +
a Nanite reference capture + a decidability statement.** No meshlet, no cluster, no DAG, no shader
that did not exist before.

**The ONE gate (from the synthesis, unchanged):**

> A reproducible per-pass Nanite cost table exists — **named GPU, named scene, named resolution,
> named error target** — together with a **stated decidability floor smaller than the delta we
> intend to claim.**

**The three kills, each a falsifiable test rather than a worry:**

| # | Kill | Test | Disposition if it fires |
|---|---|---|---|
| **K1** | **No content, no mechanism.** The corpus never approaches ~1 triangle/pixel at the intended error, so cluster LOD has no mechanism of action on our content. | R0d's census, on the owner-declared corpus and camera paths. | **Campaign refuted.** Not "descope" — the premise is gone. §9. |
| **K2** | **No baseline.** The Nanite reference cannot be produced on this box. | R0a's rig probe (before any engine code). | **Scope restatement**, an owner VALUES call: the goal becomes an **absolute** ms/quality target. §13 Q1. |
| **K3** | **Undecidable harness.** The instrument cannot resolve the delta we intend to claim. | R0e's decidability statement, with its null control. | Every future number is arguable. §9 clause 3 — and note this is the failure mode the sibling rung actually hit. |

**Falsification-first ordering.** K2 is the cheapest to test — it needs *zero* engine code and one
operator session — so R0a runs first. K1 needs the corpus and the instrument, so it lands third and
fourth. K3 needs the corpus (cost scales with density), so it lands fifth.

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

### 5.4 The statistic

Per censused frame, from the readback `(instance_id, raw_prim_id)` pairs:

1. **Triangles-per-pixel** = distinct `(instance_id, local_tri)` pairs ÷ covered pixels, where
   `local_tri = raw_prim_id % tri_count` reproduces `vb_geom_fetch.hlsli:521` on the host.
2. **Screen-space triangle-size histogram** = covered pixels per distinct
   `(instance_id, local_tri)`, bucketed by powers of two, reported as a distribution — **not** a
   mean. Sub-pixel triangles never appear in it (they lose the coverage race), so it is reported
   **together with** §5.1's submitted-triangle count, and their ratio is the micro-polygon
   indicator. The histogram alone would silently discard the population the campaign is about.
3. Both per camera path, path definitions committed as test constants — the shape
   `sv0_scene/mod.rs:149-162` already uses for its camera.

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
decidability → capture the reference.** Each rung is independently committable, has **one** gate,
and names the mutation that turns it red. *A mutation that is only argued does not count; the
commit message records the mutated run's output.*

### R0a — the reference-rig probe (zero engine code) — **kills K2 cheapest**

**Lands:** `docs/VG-R0-REFERENCE-RIG.toml` — a machine-readable record: UE version string, install
path, GPU name, driver version, capture tool + version, render resolution, `MaxPixelsPerEdge`, free
disk on the install volume, and a per-pass table for **one stock UE5 scene** (no corpus needed).
Plus `crates/boyko_app/tests/vg_r0_reference_rig.rs` reading it.

**Gate (one, three parts):** (a) every field present and not the `PENDING` sentinel — the same
sentinel discipline `goldens/PINS.toml:15` defines; (b) the recorded **GPU name matches the one
this engine reports at boot** on this box — a mechanical cross-check, not a transcription; (c) the
recorded resolution equals the census resolution constant R0c will use.

**RED if / mutation (DEMONSTRATED):** edit the recorded GPU string by one character → (b) reds.
Blank one field → (a) reds.

**If the rig cannot be supplied,** the record ships with an explicit `achievable = false` plus the
reason, the test asserts *that* shape instead, and **§9 clause 2 fires**. Recording a negative is
the rung succeeding, not failing.

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

**Gate (one, three parts):** (a) **every VB image golden byte-identical** to its `PINS.toml` pin
with the census unarmed — the usage widening and the unarmed `Option` must cost nothing;
(b) on a **procedurally generated** fixture whose screen-space triangle size is analytically known,
the census's modal bucket is the analytic bucket; (c) the census's covered-pixel total agrees with
`sv0_oracle::rasterize`'s `covered_count` on the same scene within a **pre-registered** tolerance,
fixed before the run.

**RED if / mutations (DEMONSTRATED):**
* (b): subdivide the procedural fixture 4× → the modal bucket must move by **two** buckets. A
  sensitivity control that only asserts "the number changed" is the defect this campaign keeps
  finding; the required *direction and magnitude* is what makes it a gate.
* (a): record the census copy unconditionally instead of under the `Option` → the command stream
  changes on golden frames → pins move → red.
* (c): feed the reducer the CPU oracle's own coverage instead of the readback → (c) passes
  vacuously while (b) fails; the pairing is what proves (c) is not self-referential.

### R0d — the census run — **K1's evidence**

**Lands:** the census executed over the corpus at the committed camera paths; results written to
`docs/VG-R0-DENSITY-CENSUS.md` as a table, and the **decision-bearing** numbers pinned as literals
in the test under the MEASURED discipline.

**Gate:** the census is **reproduced across 3 separate processes** with a cross-run spread within a
pre-registered bound. *The gate is that the instrument produced a reproducible number — **not**
that the number is favourable.* K1 is adjudicated in §9, deliberately, so that an unfavourable
result cannot be mistaken for a failing rung and quietly re-run until it passes.

**RED if / mutation:** point two of the three runs at different camera paths → the spread bound is
exceeded → red.

### R0e — the decidability statement — **K3's test**

**Lands:** `VbTimedPass` extended to bracket the VB raster, `vb_geo` and the classify chain, with
the written-pair bitmask of §7; a counterbalanced ABBA harness over the corpus scene with a null
control; `crates/boyko_app/tests/vg_r0_decidability.rs`, all session numbers transcribed as
literals.

**Gate (one, three parts):** (a) the null control's `|median paired delta|` is at or below its
**pre-registered** fraction of the smallest per-pass median; (b) the reported
`median_lattice_ns / |median|` is at or below its pre-registered fraction for **every** bracketed
pass — a pass whose cost sits at the lattice is reported as **not resolvable**, by name, rather
than averaged into a total; (c) the cross-session spread is within
`max(stated gate, measured lattice / |median|)`.

**RED if / mutations (DEMONSTRATED):**
* revert the harness to strict ABAB → (a) must exceed its budget. This was **measured** on this
  hardware in the sibling rung, so it is a re-demonstration, not a hope;
* make one added bracket conditional on a branch the fixture never takes → the written-pair
  bitmask assertion reds (instead of the `WAIT_BIT` readback hanging);
* halve the sample count → the confidence interval widens past the pre-registered bound → red.

### R0f — the reference capture — **closes the ONE gate**

**Lands:** the corpus imported into the R0a project; per-pass Nanite timings at the pinned error
target and the pinned resolution, for the **same camera paths**, recorded into
`docs/VG-R0-REFERENCE-RIG.toml`'s table; and the campaign's **decidability statement**: the smallest
delta this pair of instruments can jointly resolve.

**Gate:** the ONE gate of §0 — a reproducible per-pass table with named GPU / scene / resolution /
error target, **and** a stated decidability floor **smaller** than the delta the campaign intends to
claim. Both halves are literals in `vg_r0_reference_rig.rs`; the floor comes from R0e, the table
from this rung, and the test asserts the inequality between them.

**RED if / mutation:** raise the recorded floor above the intended delta → the inequality assertion
reds. That single assertion is the whole campaign's falsifiability condition, and it must be able
to fail.

---

## 9. ABORT criteria

The rung is **reverted or the campaign re-scoped** — not softened mid-flight — if any of:

1. **K1 — no content.** R0d's census shows the corpus never approaches ~1 triangle/pixel at the
   intended error target. Then cluster LOD has no mechanism of action on this content and **the
   campaign is refuted as stated**. The disposition is the owner's: change the target content class
   (and re-run R0b–R0d against it), or stop. It is explicitly *not* "generate a denser corpus" —
   §4.2 records why that makes the kill vacuous.
2. **K2 — no baseline.** R0a records `achievable = false`. Then *"faster than Nanite"* is
   unfalsifiable and the goal is restated as an **absolute** ms-at-quality target. **Owner VALUES
   call** (§13 Q1) — taken consciously, at rung one.
3. **K3 — undecidable harness.** R0e's gate reds. Then no result from this campaign is defensible,
   and the ladder does not proceed to R1 until the instrument is fixed.
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
| R9 | **Disk exhaustion masquerading as a build failure.** | This project's record: `target/` has filled this disk and surfaced as linker errors. | §11 records the measured headroom; R0a's record carries free-disk as a required field. |

---

## 11. Environment record — dated, and NOTHING READS THESE NUMBERS

Fenced exception to this document's no-measured-numbers-in-prose rule. These are facts about the
machine and the tree as of authoring; they are **evidence for design decisions, not gate
thresholds**. No test reads them, and any rung that depends on one re-derives it in its own code.

**Probed 2026-07-26, this box, working tree on branch `feat/multi-paradigm-render` at `a139799`.**

* **UE5:** no installation present. The only Epic-shaped directory on either volume,
  `D:\Epic Games`, exists and is **empty** (0 entries). No `UnrealEditor.exe` anywhere probed.
* **Free space:** `C:` 71.9 GB free of 238.3 GB; `D:` (the repo volume) 18.5 GB free of 237.7 GB.
* **Repo size:** `.git` 24.6 MB; all tracked assets under `crates/boyko_app/assets/` total 1.07 MB.
  No `.gitattributes` — **Git LFS is not configured**. No `LICENSE` file at the repo root.
* **Content today:** the VB fixtures render five instances of one `uv_sphere(radius, 28 stacks,
  40 slices)` at 512×512 (`sv0_scene/mod.rs:56-69`, `:162`). Twenty-four golden pins exist; two
  carry `sha256_hwrt = "PENDING"`.
* **Shaders:** 16 committed VB `.spv` are perturbed by a `vb_id` re-encode; 10 have a re-DXC gate.

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

**Timing:** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:63` (`PASS_COUNT = 4`), `:184-193`
(why collectors are separate), `:203` (`VbTimedPass`), `:211`/`:214`/`:219` (its three variants),
`:232` (`VB_PASS_COUNT = 3`), `:238-243` (`Option` ⇒ zero recorded commands), **`:334` (`WAIT_BIT`
blocks forever on an unwritten pair)**, `:347`/`:358`/`:371` (`Sv0TimedPass`, `SV0_PASS_COUNT = 1`).

**Harness precedent:** `crates/boyko_app/tests/sv0_deferred_term_bench.rs:20-51` (ABAB refuted by
its own null control), `:53-77` (the ABBA algebra), `:83-129` (the quantisation finding), `:284`
(`SV0_BENCH_SESSIONS`), `:286-295` (relative spread + the effective gate), `:312`
(`SV0_NULL_CONTROL_MAX_FRACTION`), `:359`/`:368`/`:377` (period / quantum / median lattice), `:389`
(the reference literal) · `crates/boyko_render/src/ui/mod.rs:87` (`FRAMES_IN_FLIGHT = 2`) ·
`crates/boyko_render/src/mesh_draw.rs:80-98` (`DrawBatch`).

**Oracles / fixtures:** `crates/boyko_app/tests/sv0_oracle/mod.rs:182-208` (`OracleVertex`,
`CoveredPixel`), `:211-256` (`Coverage`, `covered_count` at `:253`), `:279-287` (`rasterize`),
`:765-798` (`ChangedPixels`, `changed_covered_pixels`) · `crates/boyko_app/tests/sv0_scene/mod.rs:56-69`
(mesh row constants), `:149-162` (camera + `DUMP_EXTENT`), `:223` (`uv_sphere`) ·
`crates/boyko_app/tests/sv0_adequacy.rs:231-232`, `:514-515` (the shared-spawn inseparability test).

**Corpus convention:** `crates/boyko_app/assets/pbr_fixtures/README.md:1-6` ·
`.gitignore` (`/assets/materials/*` + the `!README.md` escape) ·
`goldens/PINS.toml:15` (the `PENDING` sentinel rule), `:363-364`, `:408-409` (the two unblessed
hwrt legs) · `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:196-204` (the skip shape).

---

## 13. Open questions — VALUES / SCOPE only

Performance and architecture forks are decided with numbers in this project; the format choice
(§3.3), the census instrument (§5.3) and the corpus shape (§4.2) are decided above and are not
listed here.

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
