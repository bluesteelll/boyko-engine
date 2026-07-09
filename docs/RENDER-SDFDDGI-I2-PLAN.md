# SDFDDGI I2 — Probe-Update Pass: FINAL Implementation Plan (converged)

Status: buildable, developer-executable without re-deciding architecture. Obeys
[docs/RENDER-SDFDDGI-PLAN.md](RENDER-SDFDDGI-PLAN.md) (D1-D6, Principle-0, 0%-gate, host-oracle
discipline). Produced by architect → 2 adversarial critics (perf/cost-bench-realism +
principle0/byte-identity/frozen-eDSL) → converge.

---

## Changes from critique

Every P0/P1 folded or refuted with a code-grounded reason. P2 folded or noted.

**Critic A (perf + cost-bench):**
- **P0-1 (timestamp subsystem does not exist) — FOLDED.** Grep confirmed zero `QueryPool`/`vkCmdWriteTimestamp` under `boyko_rhi_vulkan/src`. Switched the bench to **CPU wall-clock around a fenced, dispatch-only, swapchain-absent isolated submit**, with the empty-submit overhead measured once and subtracted. Corrected §5's now-wrong rejection rationale (at ~3 ms the ~20-50 µs submit overhead is <2%). No new RHI timestamp subsystem is built this rung. A timestamp-query RHI verb is explicitly deferred to a later rung as an accuracy upgrade.
- **P0-2 (no error bar) — FOLDED.** Bench now reports **median + p95 + stddev over ≥200 iterations, first 20 discarded**; the derivation rule is stated against **p95 ≤ ceiling**, not median.
- **P1-1 (16-deep edit fold cost under-stated) — FOLDED.** §5 now states the real cost model `O(subset × rays × steps × MAX_SDF_EDITS × (1 + lights_with_shadow))` explicitly; the per-light shadow march is a called-out first-class sweep axis; a "shadow OFF (N·L only)" sweep row isolates field-march from shadow-march cost. §1.2's cache note corrected to name the 16-edit inner loop.
- **P1-2 (measure shader A / ship shader B) — FOLDED.** `GI_MAX_IT` is now a **header symbol re-DXC'd per sweep value** (the SSAO-variant mechanism), so measured==shipped. The UBO-dynamic-bound path is dropped.
- **P1-3 (blend cost omitted + register spill) — FOLDED.** §5 cost model gains the blend term `O(rays × (irr_texels + depth_texels))`. Default thread mapping changed to **one-thread-per-probe-texel with groupshared cached rays** (the 448-register one-thread-per-probe default spills); register arithmetic stated.
- **P1-4 (row-strided, not contiguous) — FOLDED.** §2.4 wording corrected: tile writes are **bijective + non-overlapping** (the load-bearing D2 property, kept) but **row-strided within the layer**, not contiguous.
- **P1-5 (round-robin N∤count indexing hole) — FOLDED.** `subset_n` constrained to divisors of `DDGI_PROBE_COUNT` with a `debug_assert!`; the coarse-grid sweep uses divisor-N only. Power-of-2 defaults already satisfy it.
- **P2-1/P2-2/P2-3 — FOLDED** into §2.5 boot-barrier pin, §2.3 I4-margin note, §5 (bench on the actual 3060, no cross-class scalar).

**Critic B (principle-0 + byte-identity + frozen-eDSL):**
- **P0-1 (barrier is RDG-derived, not hand-written) — FOLDED.** Confirmed at gbuffer.rs:700-773: SSAO registers a framegraph pass (`record_graph_pass`), the RDG derives the store→load barrier at the reader, images stay GENERAL→GENERAL. §2.5 rewritten entirely in terms of the RDG seam: the update pass is a **declared framegraph pass** writing the atlas image resources; the RDG derives the update-write→resolve-read barrier at the resolve. The boot `SHADER_READ_ONLY_OPTIMAL` layout is reconciled via a graph resource seed (§2.5).
- **P0-2 (no DDGI oct_decode HLSL exists to pin against) — FOLDED.** Confirmed oct.rs has only `oct_encode_body`; goldens.rs states there is no eDSL decode. **I2 now AUTHORS the DDGI-tile `oct_decode` as a new eDSL body `oct_decode_body` in oct.rs** — host-mirrorable, the single source both I2's write-iteration and I3's read decode against. The phantom "copy the G-buffer normal decode + sync-pin against resolve" is removed.
- **P0-3 (inputs may be ringed) — REFUTED with code.** scene_types.rs:670 (`edit_list: &'a BoundBuffer`, "host-seeded ONCE before the loop") and :795 (`light_table: &'a BoundBuffer`, device-local, in-place barriered copy) confirm **neither the edit-list nor the light table is per-FIF ringed** — only the camera UBO is (`camera_ring[slot]`). The update pass binds neither the camera UBO nor any ringed input, so the single-write bind group is correct. Kept, with the per-binding ring audit table added to §7.
- **P1-1 (WAR proof overclaims uniformity under round-robin) — FOLDED.** §2.5 proof restated: invariant is *single-writer-per-tile-per-scheduled-frame* + *the RDG update→resolve barrier orders that frame's subset writes before that frame's resolve reads* + *prior-frame writes already visible via prior frames' barrier+fence chain*.
- **P1-2 (classification byte-write race) — FOLDED.** Confirmed ddgi.rs:230-232 rounds a **1-byte/probe** buffer to u32 multiples → 4 probes share a u32 → non-atomic byte stores race. Changed the classification buffer to **1 u32/probe** (8 KB, races-free); ddgi.rs sizing updated.
- **P1-3 (cross-class scalar) — FOLDED.** Derivation runs **on the actual 3060 the golden runs on**; the 3060 measurement is the binding ceiling; "2080Ti ~3 ms" is the design target, not a scaled projection. No cross-class scalar.
- **P1-4 (STORAGE re-add must not boot-fail a GI-OFF user) — FOLDED.** The format-support gate now **degrades DDGI to permanently-disabled** (does NOT fail-fast boot) when B10G11R11/RG16F STORAGE is unsupported — DDGI is opt-in, unlike always-used viewt. This diverges from `viewt_storage_format_ok` (correctly).
- **P2-1 ([[vk::image_format]] spelling) — FOLDED** into §2.2 (verify the DXC decoration string; wrong string = silent corruption).
- **P2-2 (groupshared as measured default) — FOLDED** (same as Critic A P1-3: groupshared is now the default).

---

## 0. Scope & the one number this rung exists to produce

I2 lands the **probe-update compute pass**: it sphere-traces the CSG edit-list from each probe over
a Fibonacci ray set, shades each hit (direct light + `sdf_soft_shadow_ranged` visibility), and
blends the results into the probe's octahedral irradiance tile (+ two-moment depth tile).
Subset-limited from frame one. **INTERNAL checkpoint** (strobes without hysteresis — not
owner-committable). Its true deliverable is the `ddgi_probe_update_cost` bench and the **derived**
cadence/rays/`GI_MAX_IT` under the ~3 ms ceiling. All work stays behind the GI-OFF 0%-gate: the
`grand_showcase` RTX golden `58f6c6c3d986f7a393ea53b01c5021e7360cf6f1b32bf9db05d4d8bb98999dd5` must
remain byte-identical.

---

## 1. The eDSL emitter (`boyko_shaderdsl`)

### 1.1 New modules & the frozen-span firewall

Three new leaf modules, one emitter bin, mirroring the SSAO/oct precedent:

- `crates/boyko_shaderdsl/src/probe_march.rs` — `probe_march_body<C: Cf>` — a STRIPPED analytic sphere-trace over the `field_distance` call-seam.
- `crates/boyko_shaderdsl/src/probe_blend.rs` — `probe_blend_body<C: Cf>` (one ray's cosine contribution into one oct texel) + `probe_depth_blend_body<C: Cf>` (two-moment accumulate).
- **`crates/boyko_shaderdsl/src/oct.rs` — ADD `oct_decode_body<C: Cf>`** (the missing inverse of `oct_encode_body`; the DDGI-tile decode, host-mirrorable — see §1.4). This is the P0-2 fix.
- `crates/boyko_shaderdsl/src/bin/emit_probe_update.rs` — the emitter bin (mirrors `emit_ssao_variants.rs`), **emits one HLSL variant per `GI_MAX_IT` sweep value** (32/64/96/128) into `sdf_probe_update.comp.hlsl` variants, mirroring SSAO's N-variant emission (P1-2 fix: measured==shipped).
- `emit.rs`: `emit_hlsl_probe_march()`, `emit_hlsl_probe_blend()`, `emit_hlsl_probe_depth_blend()`, `emit_hlsl_oct_decode()` (mirror `emit_hlsl_ssao()` and `emit_hlsl_oct_encode()`).

**Frozen-span firewall.** All new bodies consume `field_distance` via the existing
`Cf::call1("field_distance", ...)` seam (identical to `shadow.rs`, `normal.rs`). The frozen
`sdf_gbuffer_composite.hlsl` marcher spans and `oct_encode` are NOT touched. `probe_march_body`
does NOT call the frozen `sdf_soft_shadow_body`; the update shader's per-light glue calls
**`sdf_soft_shadow_ranged`** — copied verbatim into `sdf_probe_update.comp.hlsl` from the
already-committed `deferred_pbr.hlsl` function, pinned equal by a sync test (§6 gate 4). A shared
`.hlsli` dedup is deferred to I3 (introducing it now would touch the frozen resolve → 0%-gate risk).

### 1.2 `probe_march_body` — the stripped sphere-trace

Fixed-budget (`GI_MAX_IT`, symbolic header const) sphere-trace; the shadow marcher's `[loop]` minus
penumbra/brick/mesh-guard/gbuffer-store/accept-refine:

```
float t = GI_MINT;                              // symbolic (SHADOW_MINT class)
bool hit = false;
[loop] for (uint i = 0; i < GI_MAX_IT; ++i) {
    float d = field_distance(ro + rd * t);      // call1 seam — FROZEN field_distance;
                                                //   internally a [loop] over min(Buf[0],16) edits
    if (d < GI_HIT_EPS) { hit = true; break; }
    t = t + max(d, GI_MINT_STEP);               // R1 add form (proven byte-identical to +=)
    if (t > GI_T_MAX) { break; }
}
// out: (hit, t)   hit==false => escaped to sky
```

- `GI_MAX_IT`/`GI_MINT`/`GI_HIT_EPS`/`GI_MINT_STEP`/`GI_T_MAX` via `Cf::named_lit` (header symbols, not baked `OpConstant`) — `GI_MAX_IT` is the bench-tunable knob (§5), re-DXC'd per value.
- `hit`/`t` returned via out-cells (`Cf::RetCellF` + a bool var), the `sdf_soft_shadow_body` out-deposit shape.
- Full step `d` (not cone-step): GI rays want the true nearest-surface march.

**Cache/branch.** The *outer* march is a tight loop of 2 predictable compares, but **each
`field_distance` call is itself a `[loop]` over `n = min(Buf[0], MAX_SDF_EDITS=16)` edits** — up to
16 `load_edit` (11 SSBO word reads) + `edit_distance` + `combine` per step. The real hot loop is
16-deep, SSBO-fetch-bound per march step. The edit-list SSBO (`Buf`) is warm/resident (the marcher
reads it). This 16× multiplier is the dominant cost term and is what §5's model sizes against.

**Rejected:** the B1 over-relaxation/brick/accept-refine marcher — those refine primary-visibility
surface quality; a GI ray needs only a coarse hit + distance.

### 1.3 `probe_blend_body` — the octahedral cosine accumulate

One ray's contribution into one oct texel (decoded direction `texelDir`, ray direction `rayDir`,
radiance `L`):

```
float w = max(0.0, dot(texelDir, rayDir));
sum_rgb += L * w;
sum_w   += w;
```

Tile write (after all rays): `irr = sum_rgb / max(sum_w, GI_MIN_SUM_WEIGHT)` reusing the
resolve-side `DDGI_MIN_SUM_WEIGHT = 1e-6` (I0b goldens). `dot` = inline component reads
(`Cf::vec3_x/y/z` + mul/add) — no `vec3_dot` leaf, no frozen printer fork (SSAO discipline). Update
is GPU-golden + tolerance (not bit-exact — latitude granted), so no host oracle required this rung.

### 1.4 `oct_decode_body` + the encode/decode consistency seam (P0-2 fix)

**What.** Author `oct_decode_body<C: Cf>` in oct.rs as the mathematical inverse of the committed
`oct_encode` fold (`e*2-1` → unfold `abs`/sign → saturate → normalize — the same math already
present as the *G-buffer normal* decode in `sdf_ssao.comp.hlsl:159-166`, but now authored as a
**first-class eDSL body** so it is host-mirrorable and single-sourced). Emitted into
`sdf_probe_update.comp.hlsl` between `// === GENERATED oct_decode BEGIN/END ===` sentinels.

The update pass iterates **per texel**: `texelUV → texelDir = oct_decode(texelUV)`, weights all
cached rays against `texelDir`. Only `oct_decode` is needed on the write path — no `oct_encode`
(per-texel gather, not per-ray scatter).

**The I2→I3 contract (load-bearing).** I2 is the FIRST to commit a DDGI-tile `oct_decode`. I3's
resolve MUST decode against this **exact eDSL-emitted `oct_decode`** (same committed HLSL text). The
tile-UV↔texel remap and spacing reconstruction that I3 owns MUST live in the *texel→UV chain
OUTSIDE the decode*, never inside `oct_decode` itself — otherwise I2's writes and I3's reads desync.
This contract is stated as a prominent header comment in oct.rs and in `sdf_probe_update.comp.hlsl`.
A host mirror `goldens::oct_decode` already exists (I0b) and is pinned equal to the new eDSL body by
a sync test (§6 gate 4 — `oct_decode_edsl_matches_host`, NOT the phantom `_matches_resolve`).

**Why author it (not reuse the G-buffer normal decode text).** Authoring the eDSL body makes the
decode host-mirrorable and provable at I2, eliminating the encode/decode-mismatch class *now* rather
than hand-waving to I3. Reusing the G-buffer-normal decode text would pin against a function with a
different downstream chain (I3's tile-UV remap) that the normal decode lacks — the exact
silent-desync trap.

### 1.5 `probe_depth_blend_body` — the two-moment tile

Per depth texel: `dmean += w·t`, `dmean2 += w·t²` over rays (same cosine `w`, `t` = marched hit
distance or `GI_T_MAX` on sky miss). Written `(dmean/dw, dmean2/dw)`. No `pow` this rung (the
depth-sharpen `pow` is an I4 Chebyshev knob). Depth tile has its own 14×14-valid/16-tile geometry →
its own texel-iteration loop with `ddgi_probe_tile_origin(x,y,z, DDGI_DEPTH_TILE_EDGE)`.

### 1.6 The committed shader `sdf_probe_update.comp.hlsl`

New file `crates/boyko_rhi_vulkan/shaders/sdf_probe_update.comp.hlsl` (+ committed `.comp.spv` per
`GI_MAX_IT` variant), structured like `sdf_ssao.comp.hlsl`:

- Resource decls (§2.2) with `[[vk::image_format]]` pins on the two STORAGE atlas images (verify the DXC spelling for B10G11R11 — P2-1).
- `StructuredBuffer<uint> Buf : register(t0)` then `#include "sdf_field.hlsli"` (the include contract).
- The verbatim `sdf_soft_shadow_ranged(...)` (pinned by §6 gate 4).
- The GENERATED `oct_decode` span (§1.4).
- Hand-written glue: probe-index→world position; the round-robin subset gate (§4); classification read/write (§4); Fibonacci ray fetch (§2.3); the per-ray direct-light shade loop (iterate `LightBuf`, `sdf_soft_shadow_ranged` visibility); the two texel-iteration loops (irr 6×6, depth 14×14) splicing the GENERATED `probe_blend`/`probe_depth_blend` spans; the GENERATED `probe_march` span inside the per-ray loop.
- `[numthreads(...)]` per §2.4 (groupshared cooperative — default).

---

## 2. The update compute pass (RHI + host wiring)

### 2.1 Own pipeline layout (D4)

A dedicated `ComputePipeline` + `VulkanBindGroupLayout`, separate from the resolve set (which is at
the 19/19 cap). Load via new `sdf_probe_update_spirv(gi_max_it) -> &'static [u32]` in `compute.rs`
selecting the re-DXC'd variant (`include_bytes!` + const-asserted size, mirroring `sdf_ssao_spirv`).
Set 0 = §2.2 bindings; no push range (params ride the UBO — matches SSAO reading camera from UBO, no
push).

### 2.2 Exact bindings (update set 0)

| reg | HLSL decl | kind | access | source | ringed? |
|-----|-----------|------|--------|--------|---------|
| t0 | `StructuredBuffer<uint> Buf` | Storage (RO) | read | `scene.edit_list` (`sdf_field.hlsli` contract) | **NO** (scene_types.rs:670, host-seeded once) |
| u1 | `[[vk::image_format("<B10G11R11 spelling>")]] RWTexture2DArray<float4> gIrrOut` | Storage image (RW) | write | `DdgiAtlas::irradiance()` | NO (D1) |
| u2 | `[[vk::image_format("rg16f")]] RWTexture2DArray<float2> gDepthOut` | Storage image (RW) | write | `DdgiAtlas::depth()` | NO (D1) |
| t3 | `RWStructuredBuffer<uint> Classification` | Storage (RW) | read+write | `DdgiAtlas::classification()` (now **u32/probe**, P1-2) | NO |
| t4 | `StructuredBuffer<float4> RayTable` | Storage (RO) | read | Fibonacci ray-table device buffer (§2.3) | NO (boot-static) |
| t5 | `StructuredBuffer<uint> LightBuf` | Storage (RO) | read | `scene.light_table` | **NO** (scene_types.rs:795, device-local in-place) |
| b6 | `cbuffer DdgiUpdate` | Uniform | read | dedicated update UBO (§2.3) | NO (I2 static; see §7) |

**One bind group, written ONCE (no per-FIF ring).** The per-binding audit (rightmost column)
confirms every input is non-ringed — `edit_list` and `light_table` are single `&'a BoundBuffer`s
host/device-seeded once and updated in place via barriered copies (the RDG orders those copies);
only the camera UBO is ringed and the update pass does not bind it. So a single bind group captures
no stale slot.

`BindGroupEntry` kinds (mirror targets.rs): `StorageBuffer`(t0), `StorageImage`(u1,u2),
`StorageBuffer`(t3,t4,t5), `UniformBuffer`(b6).

### 2.3 The update UBO + Fibonacci ray table (Principle-0)

**UBO (`b6`).** New `#[repr(C)]` `DdgiUpdateUbo`: `origin: [f32;4]`, `inv_spacing_dims: [u32;4]`
(bit-cast like `ResolvedDdgi`), `frame_index: u32`, `subset_n: u32`, `rays_per_probe: u32`,
`light_count: u32`. Written per-frame host-coherent; the DEVICE UBO is RHI-owned (the
`ResolvedDdgi`→resolve-UBO split). `frame_index` host-frame-derived. **I2 ships identity
ray-rotation → the UBO is effectively static → no ring** (strobes accepted, internal checkpoint). I4
turns on the quaternion rotate; carry a small margin in the derived cadence for its per-ray mul/add.

**Ray table.** A DEVICE storage buffer of `rays_per_probe` `float4`s (xyz = unit spherical-Fibonacci
direction), CPU-precomputed at boot and uploaded ONCE (the `MeshSdfTexture::upload_region`
boot-submit shape). Per-frame decorrelation = an in-shader quaternion rotate (D6, mul/add only),
identity at I2, rotation slot wired for I4. **Principle-0:** durable DEVICE buffer, not a per-frame
host Vec. Owned in a new `DdgiUpdateResources` (Resource-owned carrier, §7).

**Rejected:** in-shader trig Fibonacci gen (transcendental, per-thread-redundant) — boot-baked table
is one SSBO fetch/ray and bit-stable.

### 2.4 Thread mapping & dispatch dims (DEFAULT = groupshared cooperative; P1-3/P2-2 fix)

**Default: one thread-block per probe, threads cooperate over texels via groupshared cached rays.**
`[numthreads(64,1,1)]`, one block per active probe; dispatch `groups_x = active_subset_probe_count`.
Per block:

1. block→probe index in the current subset (§4); read classification; if INACTIVE, block early-returns.
2. compute probe world position.
3. **cooperatively march the `rays_per_probe` rays** (thread `i` marches rays `i, i+64, …`), writing each result `(dir, L, t)` into **groupshared** arrays (`gs_dir[R]`, `gs_L[R]`, `gs_t[R]`, R = rays_per_probe ≤ 128 → 128×7 floats = 3.5 KB groupshared, well within 32 KB). `GroupMemoryBarrierWithGroupSync()` after.
4. **cooperatively gather:** the 36 irradiance texels + 196 depth texels are partitioned across the 64 threads; each thread `oct_decode`s its texels and loops the groupshared cached rays (the spliced `probe_blend`/`probe_depth_blend` spans), writing its texels to the STORAGE images.
5. one thread sets the converged-once bit (§4).

**Register arithmetic (why groupshared, not one-thread-per-probe).** One-thread-per-probe caches
R=64..128 ray results in *registers* = 64×7…128×7 = 448…896 registers/thread → hard spill, craters
occupancy. Groupshared moves the ray cache to LDS (3.5 KB), keeps live registers at
O(texels-per-thread) ≈ a handful, and gives full occupancy. This is the measured default;
one-thread-per-probe is the documented fallback only if the bench shows LDS bank conflicts dominate
(unlikely).

**Cache/store behavior.** Atlas writes: each block writes ONLY its own probe's tile — **bijective +
non-overlapping** (the load-bearing D2 property, proven by ddgi.rs `probe_tile_origin_is_bijective`),
but **row-strided within the array layer** (tile width 8/16, layer row stride 128/256), NOT
contiguous. The non-overlap (not contiguity) is what defeats intra-dispatch WAR. Field reads:
streaming over the warm edit-list SSBO per march step.

### 2.5 The RDG-declared update→resolve barrier (P0-1 + P1-1 fix)

**What (on the real mechanism).** The engine does NOT hand-write `cmd_pipeline_barrier` for compute
passes — confirmed at gbuffer.rs:700-773, where SSAO registers a framegraph pass
(`record_graph_pass(ssao_pass, …)`) and the RDG derives the store→load barrier at the reader, keeping
images GENERAL→GENERAL. The update pass follows this exactly:

1. **Declare an RDG pass** `ddgi_update` in `graph_bridge.rs` (mirror the `ssao` pass declaration), declaring the two atlas images as graph resources with a **WRITE (storage) access** and the edit-list/light-table/ray-table/classification buffers as READ. Because the atlas is boot-initialized to `SHADER_READ_ONLY_OPTIMAL` (ddgi.rs) — unlike SSAO's UNDEFINED first-touch — **seed the atlas image resources with `add_image_seeded(SHADER_READ_ONLY_OPTIMAL)`** (the light_table `add_buffer_seeded` cross-frame-seed pattern, graph_bridge.rs:306) so the RDG knows the real starting layout and derives the correct `SHADER_READ_ONLY_OPTIMAL → GENERAL` (or GENERAL-throughout) transition for the storage write.
2. In gbuffer.rs, record the pass ONLY inside `if let Some(activation) = &scene.ddgi_update` (mirror the `scene.ssao` block): `record_graph_pass(ddgi_update_pass, …)` (RDG emits the input barriers), then bind pipeline + the single (non-ringed) bind group + `cmd_dispatch(active_subset_probe_count, 1, 1)`.
3. The **update-write → resolve-read barrier is DERIVED BY THE RDG at the resolve** (the atlas reader), exactly as SSAO's store→load is derived at the resolve — NOT hand-written here. Both producer and consumer are COMPUTE (the deferred resolve is a compute dispatch), so the RDG inserts a COMPUTE→COMPUTE / SHADER_WRITE→SHADER_READ barrier on both atlas images.

**Proof it defeats the D2 WAR / cross-frame race (restated for round-robin).**
- **Intra-dispatch:** each block writes only its own probe's tile (bijective/non-overlap, §2.4) and never image-loads a tile within the dispatch (accumulators are groupshared/registers). No in-dispatch RAW/WAR on the atlas.
- **Round-robin cross-frame invariant (the honest statement):** a tile is written by exactly one block on its scheduled frame; the RDG update→resolve barrier orders *that frame's subset writes* before *that frame's resolve reads*; the (N-1)/N tiles NOT updated this frame were last written K frames ago and are already visible via the intervening frames' RDG barriers + the FIF fence chain. The grid is world-fixed (D1) → probe *i* is the same world point every frame → no reprojection, no per-FIF atlas ring; the atlas is device-only (no host-mapped ring) → no host-write-before-fence hazard (the "wrong-only-in-motion" class). D1+D2 collapse the cross-frame race to nothing.
- **Boot-barrier pin:** the first-frame `SHADER_READ_ONLY_OPTIMAL → GENERAL` transition the RDG derives is a WAR on the boot clear (ddgi.rs boot-transition), correctly ordered because the boot submit is fence-waited before frame 0. Pin this against the boot barrier in a comment.

**Rejected:** hand-written `cmd_pipeline_barrier` transitions — they would double-transition against
the RDG's derived barriers (validation error / undefined-layout race). The RDG is the sole barrier
authority here.

---

## 3. STORAGE re-add + the format-support boot gate (P1-4 fix: degrade, don't crash)

**Re-add STORAGE.** In `ddgi.rs::DdgiAtlas::create`, add `ImageUsage::STORAGE` to BOTH atlas `usage`
(ddgi.rs:201 and :217) → `SAMPLED | TRANSFER_DST | STORAGE`. Update the two doc notes ("NO STORAGE at
I1 … arrive together at I2" → done). The TRANSFER_DST boot-clear/transition path (ddgi.rs:282-290) is
UNCHANGED — STORAGE usage does not perturb the clear (state this explicitly for the golden argument).

**The format-support gate — GRACEFUL DEGRADATION, not fail-fast.** Mirror the *query* of
`viewt_storage_format_ok` (device.rs) but NOT its fail-fast. DDGI is opt-in; viewt is always used —
so:
- Add two `DeviceCaps` bools: `ddgi_irr_storage_ok` (B10G11R11_UFLOAT_PACK32 + `VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT`, OPTIMAL tiling) and `ddgi_depth_storage_ok` (R16G16_SFLOAT + STORAGE_IMAGE_BIT), queried in `query_device_caps` via `vkGetPhysicalDeviceFormatProperties`.
- **Do NOT fail-fast boot.** Instead, if either is false, force DDGI permanently disabled: `resolve_ddgi_grid` reads the caps and clamps `ResolvedDdgi` to DISABLED (all-zero) regardless of `DdgiConfig::enabled()`. The atlas is still created **without STORAGE** in that case (fall back to `SAMPLED | TRANSFER_DST`, the I1 usage) so `vkCreateImage` cannot panic. A GI-OFF (or unsupported-device) user boots normally; the 0%-gate already makes disabled DDGI cost nothing.
- **No new `BootError` variants.** Add one debug-log line ("DDGI disabled: B10G11R11/RG16F storage unsupported").

**Rejected:** `shaderStorageImageWriteWithoutFormat` + untyped writes — every existing storage image
pins `[[vk::image_format]]`; staying consistent avoids a device-feature toggle. **Rejected:**
fail-fast — it would turn a currently-booting GI-OFF device into a boot-failing one for an opt-in
feature the user never requested.

---

## 4. Round-robin subset limiting + classification (P1-2 + P1-5 fix)

**Subset selection.** UBO carries `subset_n` (N; default 4 per D3, bench-set) + `frame_index`. Probe
`p` is in the subset iff `(p % subset_n) == (frame_index % subset_n)`. Dispatch sized to the subset:
`active_count = DDGI_PROBE_COUNT / subset_n` (exact division — see the constraint), block `b` maps to
`probe_index = b * subset_n + (frame_index % subset_n)`.

**`subset_n` MUST divide `DDGI_PROBE_COUNT` (P1-5 fix).** `DDGI_PROBE_COUNT = 16·8·16 = 2048 =
2^11`, so any power-of-2 N divides it; the defaults {1,2,4,8} all satisfy it. The coarse-grid sweep
(§5) uses grids whose probe counts are likewise divisible by the swept N. Guard:
`debug_assert!(DDGI_PROBE_COUNT % subset_n == 0)` on the host + a compile-time `const_assert` on the
default. This closes the ragged-residue-class hole (some probes never updated → dark patches).

**Classification (GPU buffer, 1 u32/probe — P1-2 fix).** Change the classification buffer from 1
byte/probe (ddgi.rs:230-232, 4-per-u32 → byte-store race) to **1 u32/probe** (`DDGI_PROBE_COUNT * 4`
= 8 KB — trivial). Then each probe owns a full u32 → plain non-atomic stores are race-free
(single-writer-per-element-per-frame holds at word granularity). Fields:
- **bit0 = active.** Re-evaluated each scheduled frame: `inside = field_distance(probe_pos) < GI_INSIDE_EPS`; write `active = !inside`. Cheap (one field eval, amortized over rays). Geometry is dynamic → re-evaluate, don't freeze.
- **bit1 = converged-once.** Set on first successful tile write (gates I3/I5 sky fallback). I2 sets; I3 reads.

Update ddgi.rs sizing note + the boot fill-clear (still `vkCmdFillBuffer`, now over `PROBE_COUNT*4`
bytes, cleared to 0 = unconverged/inactive-until-proven).

**Why GPU buffer, not host Vec.** Principle-0 + the SP4-race lesson. Single-writer-per-u32-per-frame
→ no atomics needed.

---

## 5. The `ddgi_probe_update_cost` bench harness (P0-1, P0-2, P1-1, P1-2, P1-3 fixes)

**Measurement method (P0-1 fix): CPU wall-clock around a fenced, dispatch-ONLY, swapchain-absent
isolated submit.** No timestamp-query RHI subsystem exists (grep-confirmed zero
`QueryPool`/`vkCmdWriteTimestamp`), so building one is out of scope this rung. Instead a new
`#[ignore]` measurement test `crates/boyko_rhi_vulkan/tests/ddgi_probe_gi_cost.rs` (named
`ddgi_probe_gi_cost`, NOT `ddgi_probe_update_cost` — a test/exe name containing "update" trips
Windows installer-detection UAC elevation, os error 740, on this box) (run
`--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`):
1. Boot a device (offscreen, no swapchain acquire/present); allocate atlas + classification(u32) + ray table + the grand_showcase edit-list SSBO (the real CSG fold cost).
2. Create the update pipeline (the `GI_MAX_IT` variant under test).
3. Record a command buffer containing ONLY reset→bind→dispatch→(RDG barrier); `vkQueueSubmit` + `vkWaitForFences`; wall-clock the submit+wait.
4. **Measure the empty-submit overhead once** (an identical fenced submit with a no-op/zero-dispatch command buffer) and **subtract it** from every measurement. At the ~3 ms target this fixed overhead (~20-50 µs) is <2%.
5. **≥200 iterations, discard the first 20; report median + p95 + stddev** (P0-2 fix — error bar for the knife-edge decision).
6. **Cross-check:** assert the empty-submit overhead is stable (stddev < 10% of it) or flag the box as noisy.

**Rejected:** `vkCmdWriteTimestamp` — requires a from-scratch RHI query subsystem that does not
exist; a criterion bench crate — no render/rhi bench harness exists and criterion's CPU loop is wrong
for GPU work. A timestamp-query RHI verb is a deferred accuracy upgrade for a later rung.

**Cost model (P1-1 + P1-3 fix — stated so the developer sizes the sweep):**
```
march_cost  = subset × rays × GI_MAX_IT × MAX_SDF_EDITS          (primary field march)
shadow_cost = subset × rays × lights_with_shadow × GI_MAX_IT × MAX_SDF_EDITS   (per-light ranged shadow march)
blend_cost  = subset × rays × (IRR_TEXELS[36] + DEPTH_TEXELS[196])   (the texels×rays gather, per §2.4)
total ≈ march_cost + shadow_cost + blend_cost
```
Worked midpoint (N=4→512 probes, 64 rays, GI_MAX_IT=64, 1 shadowed light): march ≈ 512·64·64·16 =
33.5M folds; shadow ≈ another 33.5M; blend ≈ 512·64·232 = 7.6M dots. `shadow_cost` is a **first-class
sweep axis** (each shadowed light adds a full `GI_MAX_IT×16` march per ray — the dominant
multiplier), and the blend is a distinct term, not an afterthought.

**Sweep grid (all knobs first-class):**
- `rays_per_probe` ∈ {16,32,64,128} (UBO field + ray-table size).
- `subset_n` ∈ {1,2,4,8} (UBO field; N=1 = full-grid ceiling test; all divide 2048, P1-5).
- `GI_MAX_IT` ∈ {32,64,96,128} (**header symbol → re-DXC per value**, the SSAO-variant mechanism → measured==shipped, P1-2).
- grid ∈ {16·8·16 owner-locked default, coarser (e.g. 12·6·12) to quantify the coarsen-grid escape hatch}.
- **light/shadow rows:** {N·L only (shadow OFF), 1 shadowed directional, +1 punctual shadowed} — the "shadow OFF" row **isolates field-march from shadow-march cost** (P1-1) so the budget can be attributed.

**Cadence derivation (P0-2 + P1-3, run on the 3060):**
- The bench runs on the **actual RTX 3060 the golden runs on**; the 3060 p95 is the binding ceiling. "2080Ti ~3 ms" is the *design* target the 3060 measurement must not exceed by more than a documented margin — **no cross-class scalar** (the compute is SSBO-fetch-bound, where FLOP ratios mislead).
- Rule: pick the largest `(rays_per_probe, GI_MAX_IT, grid, shadowed-lights)` whose **per-frame p95** cost = `full_grid_p95 / subset_n` ≤ the 3060 ceiling, preferring more rays (quality) over lower N (latency); set `DdgiConfig`/UBO defaults + the shipped header `GI_MAX_IT` variant to those.
- If even {N=8, 16 rays, GI_MAX_IT=32, coarse grid, 1 light} exceeds the ceiling at p95 → escalate (a VALUES call: budget/grid was locked assuming feasibility; §10 open-risk #1). Otherwise the knobs absorb it — no VALUES call.
- **Escape-hatch knobs wired first-class** so derived numbers are settable without code: `rays_per_probe`/`subset_n` = UBO fields; `GI_MAX_IT` = the shipped re-DXC variant; grid = `DdgiConfig` (D5). Brick-cache-for-GI-rays is a deferred later lever (I2 marches the raw field).

---

## 6. The 0%-gate proof + my gates

**GI-OFF skips the dispatch entirely.** The update pass is recorded ONLY inside `if let
Some(activation) = &scene.ddgi_update` in gbuffer.rs (mirror the `scene.ssao.is_some()` skip).
`scene.ddgi_update` is `None` whenever `ResolvedDdgi` is disabled (default `ddgi_indirect=false`, OR
the format gate forced-disabled per §3). When `None`: no RDG pass recorded, no seed, no bind, no
dispatch → the command stream is byte-identical to the pre-I2 path. The atlas/classification/ray-table
are allocated regardless (like the SSAO image) but stay in boot `SHADER_READ_ONLY_OPTIMAL`, unread
(I3's read is also gated on `ddgi_mode != 0`). The grand golden hashes the IMAGE → an
allocated-but-unread atlas is image-invisible.

**One predicate, three consumers.** `boyko_render`'s ddgi plugin builds `scene.ddgi_update =
Some(...)` ONLY when `ResolvedDdgi::enabled()` (the SAME predicate that sets the LightBuf word-7 bit-4
gate, `sync_ddgi_light_gate`) — so the LightBuf gate, the resolve read, and the update dispatch
cannot disagree.

**My gates (in order):**
1. `cargo check --all-targets`.
2. `cargo clippy` on touched sources (touch LastWriteTime first — the false-fresh machine fact).
3. eDSL emit tests: `probe_march_matches_edsl_emit`, `probe_blend_matches_edsl_emit`, `probe_depth_blend_matches_edsl_emit`, `oct_decode_matches_edsl_emit` (`.contains` drift gate) + re-DXC byte-identity of each `sdf_probe_update.comp.hlsl` `GI_MAX_IT` variant → committed `.spv` (or `.contains` + committed-`.spv` length const-assert if DXC absent).
4. Sync tests: `sdf_soft_shadow_ranged_copy_matches_resolve` (update's copied fn == `deferred_pbr.hlsl`) + **`oct_decode_edsl_matches_host`** (the new eDSL `oct_decode_body` == `goldens::oct_decode` host mirror — NOT the phantom `_matches_resolve`, P0-2).
5. The `grand_showcase` RTX golden == `58f6c6c3d986f7a393ea53b01c5021e7360cf6f1b32bf9db05d4d8bb98999dd5` (`--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`).
6. `ddgi_probe_update_cost` (§5): run, capture median/p95/stddev, derive + set defaults, re-run gate 5 to confirm byte-identity after resetting `DdgiConfig` to disabled default.

**Rejected:** record-always + in-shader `ddgi_mode` branch — a recorded dispatch changes the command
stream (breaks byte-identity even if the shader no-ops). The SSAO skip-the-whole-block precedent is
the proven 0%-gate.

---

## 7. Integration & sequencing

**ECS/plugin wiring (`boyko_render`).**
- Extend `ddgi_plugin.rs`: a system that, when `ResolvedDdgi::enabled()`, populates the host-side update activation (pipeline/layout borrows + the UBO write) into the scene bundle; else leaves it `None`. Ordered `.after_set(DdgiResolveSet)` so `ResolvedDdgi` is written before the update UBO reads it.
- New Resource `DdgiUpdateResources` (RHI-owned handles: update pipeline+layout+bind group, ray-table buffer, update UBO) — created at device-boot alongside `DdgiAtlas`, torn down with it. Resource-owned carrier (Principle-0), not a side store.

**RHI/present wiring (`boyko_rhi_vulkan`).**
- `scene_types.rs`: add `DdgiUpdateActivation<'a>` (`pipeline: &ComputePipeline`, `layout: &VulkanBindGroupLayout`, mirror `SsaoActivation`) + `pub ddgi_update: Option<DdgiUpdateActivation<'a>>` on `GBufferScene`.
- `graph_bridge.rs`: declare the `ddgi_update` RDG pass; **seed the two atlas images with `add_image_seeded(SHADER_READ_ONLY_OPTIMAL)`** (the `add_buffer_seeded` cross-frame pattern @306) so the RDG derives the correct storage-write transition from the real boot layout.
- `targets.rs`: write the update bind group ONCE (mirror `ssao_set` but **NOT** `[FRAMES_IN_FLIGHT]`: single, per the §2.2 ring audit; all inputs non-ringed).
- `gbuffer.rs`: the `if let Some(&scene.ddgi_update)` block AFTER the marcher (edit-list warm) and AFTER the L0 light-table copy (`LightBuf` COMPUTE-read-visible — the same ordering the cull relies on), BEFORE the resolve. `record_graph_pass(ddgi_update_pass, …)` → bind → `cmd_dispatch(active_subset_probe_count,1,1)`.

**Per-binding ring audit (Critic B P0-3 — the demanded table):**

| binding | source | ringed in current wiring? | evidence |
|---------|--------|---------------------------|----------|
| t0 edit-list | `scene.edit_list` | NO | scene_types.rs:670, `&'a BoundBuffer`, "host-seeded ONCE before the loop" |
| t5 light-table | `scene.light_table` | NO | scene_types.rs:795, device-local, in-place barriered copy (graph_bridge light_upload) |
| u1/u2 atlas | `DdgiAtlas` | NO | D1 world-fixed, single-atlas |
| t3 classification | `DdgiAtlas` | NO | single device buffer |
| t4 ray-table | `DdgiUpdateResources` | NO | boot-static |
| b6 update UBO | `DdgiUpdateResources` | NO (I2 static) | identity rotation → no per-frame change |

→ single bind group is correct; the only ringed engine buffer (camera UBO, `camera_ring[slot]`) is
not bound by the update pass.

**Ordering vs marcher/resolve.** marcher → **ddgi update** → resolve. The update does not depend on
the G-buffer (it marches from probes, not screen pixels), but is placed after the marcher to keep the
edit-list SSBO warm and match the barrier flow. The RDG update→resolve barrier orders the atlas write
before I3's read.

**Per-FIF confirmation (D1).** CONFIRMED no per-FIF ring: world-fixed grid (D1), persistent-single
atlas (D2), probe *i* = same world point every frame → no reprojection, no cross-frame WAR (§2.5).
The only per-frame input is `frame_index`/rotation; I2 ships identity → static UBO → no ring this
rung.

---

## 8. New/changed files (developer checklist)

**New:**
- `crates/boyko_shaderdsl/src/probe_march.rs` — `probe_march_body`.
- `crates/boyko_shaderdsl/src/probe_blend.rs` — `probe_blend_body`, `probe_depth_blend_body`.
- `crates/boyko_shaderdsl/src/bin/emit_probe_update.rs` — the emitter (N `GI_MAX_IT` variants).
- `crates/boyko_rhi_vulkan/shaders/sdf_probe_update.comp.hlsl` (+ committed `.comp.spv` per `GI_MAX_IT` variant).
- `crates/boyko_render/src/ddgi_update.rs` — `DdgiUpdateResources`, `DdgiUpdateUbo`, the Fibonacci-table builder, the activation-populate system.
- `crates/boyko_rhi_vulkan/tests/ddgi_probe_update_cost.rs` — the bench.
- `crates/boyko_shaderdsl/tests/emit_probe_update.rs` — emit drift + sync tests.

**Changed:**
- `crates/boyko_shaderdsl/src/oct.rs` — **ADD `oct_decode_body`** + the I2→I3 decode-contract comment.
- `crates/boyko_shaderdsl/src/lib.rs` — `pub mod probe_march; pub mod probe_blend;`.
- `crates/boyko_shaderdsl/src/emit.rs` — `emit_hlsl_probe_march/_blend/_depth_blend/_oct_decode`.
- `crates/boyko_rhi_vulkan/src/ddgi.rs` — STORAGE re-add to both atlases **conditional on the format caps** (§3 degrade path); classification buffer → **u32/probe** (P1-2); update the doc notes + sizing.
- `crates/boyko_rhi_vulkan/src/device.rs` — 2 `DeviceCaps` bools + `query_device_caps` format queries (NO fail-fast, §3).
- `crates/boyko_render/src/ddgi_config.rs` — `resolve_ddgi_grid` clamps to DISABLED when caps are false (§3 degrade).
- `crates/boyko_rhi_vulkan/src/compute.rs` — `sdf_probe_update_spirv(gi_max_it)` (variant select + size const-assert).
- `crates/boyko_rhi_vulkan/src/present/scene_types.rs` — `DdgiUpdateActivation` + `GBufferScene.ddgi_update`.
- `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` — declare `ddgi_update` RDG pass + `add_image_seeded(SHADER_READ_ONLY_OPTIMAL)` atlas seeds.
- `crates/boyko_rhi_vulkan/src/present/targets.rs` — write the update bind group (single, no ring).
- `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` — the gated `record_graph_pass` update block.
- `crates/boyko_render/src/ddgi_plugin.rs` — the activation-populate system, `.after_set(DdgiResolveSet)`.

---

## 9. Mandatory tests & debug_asserts

**Unit (CPU, GPU-free):** `probe_march_body::<EvalCf>` hits a unit sphere at expected `t` / escapes
to sky; `probe_blend_body::<EvalCf>` normalizes (all-equal rays → uniform irradiance; single-direction
ray → cosine peak at that texel); `oct_decode_body::<EvalCf>` == `oct_encode` inverse round-trip AND
== `goldens::oct_decode`; the subset residue-class map covers every probe exactly once over
`subset_n` frames (bijectivity, like ddgi.rs's tile-origin test); the Fibonacci table is unit-length +
evenly distributed.

**GPU (`#[ignore]`, RTX):** the update dispatch runs clean under validation (RDG barrier
correctness); `probe_march` GPU hit-distance ≈ host within tolerance (GPU-golden + tolerance); GI-OFF
command-stream byte-identity (the `58f6c6c3` golden).

**Emit:** `.contains` drift gates + re-DXC byte-identity (§6 gate 3-4).

**debug_assert! invariants:** `probe_index < DDGI_PROBE_COUNT` after the residue map; `subset_n >= 1
&& DDGI_PROBE_COUNT % subset_n == 0` (P1-5); `rays_per_probe == RayTable length`; tile-origin fits the
atlas (already in ddgi.rs); `sum_w >= 0` before the divide-guard.

---

## 10. Open questions (none require a VALUES call unless flagged)

1. **Bench feasibility (the one reserved VALUES escalation).** IF the 3060 p95 exceeds the ceiling even at the coarsest knobs {N=8, 16 rays, GI_MAX_IT=32, coarse grid, 1 light} → the owner chooses: relax the budget, shrink the grid below owner-locked, or cut rays below usable quality. Plan open-risk #1 reserves this; everything short of it the knobs absorb (no VALUES call).
2. **Groupshared LDS bank conflicts (§2.4).** Default = groupshared cooperative (register-fit proven); fallback = one-thread-per-probe (only if the bench shows LDS conflicts dominate — unlikely). A perf fork decided from the numbers, not a VALUES call.
3. **B10G11R11 `[[vk::image_format]]` DXC spelling (P2-1).** The developer must confirm the exact DXC decoration string for B10G11R11_UFLOAT against the Vulkan format-feature the boot gate queries; a wrong string is silent write-corruption, so pin it with a round-trip GPU test (§9 GPU).

All other decisions are locked above with rationale + rejected-alternative. No VALUES call is needed
to begin implementation.
