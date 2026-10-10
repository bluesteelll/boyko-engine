# Dynamic materials — the survey

> Status: research survey, 2026-09-25, architect **pass 2**: revised after the first architecture
> critique. The design that rests on it is
> [`DYNAMIC-MATERIALS-DESIGN-SPACE.md`](DYNAMIC-MATERIALS-DESIGN-SPACE.md); its closing *Review log*
> lists every remark and where it landed in both documents. Pass 2 corrected the tree census
> (§1.1, §1.6, §1.9), added two live defects the critique traced (§1.11), and corrected four numbers
> (the 3060/3070 bandwidth ratio, the Hable scaling, the f32 phase error, the `SdfEdit` free lanes).
>
> **How it was checked.** Every repository claim was re-opened at trunk commit **`6394bc5e`** through
> the `D:/wt/docs` worktree (branch `u/research-0925`, identical to the trunk). A `path:line` appears
> only where that line was re-read by content at that commit; everywhere else the citation is a path
> plus a symbol. No `cargo` command was run and nothing was timed for this document. `docs/render/` is
> not in the scanned list of `tests/internal_docs_anchors.rs`, so these anchors are checked by hand,
> not by CI.
>
> **Scope.** The workflow assigned this survey to *dynamic materials*: time-varying, per-instance and
> programmable surface appearance. The requesting message also names post-FX and anti-aliasing,
> effects such as explosions and rain, sun rays, and volumetrics. Those are separate research
> threads. This survey touches them only where they meet a material: rain wetness, explosion
> flipbooks, hit flash and dissolve.
>
> **Web budget.** The upstream gathering session used its whole web-search budget (200 of 200
> searches) partway through. This pass could only fetch known URLs directly. With that it
> re-verified the load-bearing external claims:
> - Hable's timings;
> - NVIDIA's `VK_EXT_device_generated_commands` driver line;
> - the Frostbite GDC 2007 deck;
> - the RedLynx SIGGRAPH 2015 deck;
> - NVIDIA's RTX 3060 and RTX 3070 spec pages.
>
> Pass 2 had no search budget left either. It fetched these known URLs:
> - the Destiny TFX GDC 2017 page;
> - the CUDA programming guide's compute-capability table;
> - Wikipedia's GeForce 30 series table, for the memory data rate the NVIDIA pages omitted;
> - two Epic documentation pages (instanced static meshes; constant material expressions).
>
> **Decima is not covered** (§2.6), and several other gaps are listed in §5.

**Provenance markers used throughout:**
- **[P]** primary: vendor documentation, the talk's own slides, a paper, engine source.
- **[S]** secondary: a practitioner blog, or notes on someone else's talk.
- **[V]** vendor claim: a number the vendor measured on its own hardware or driver.
- **[E]** estimate or derivation made here; the arithmetic is shown next to it.
- **[T]** tree fact at `6394bc5e`.

**Reference GPU.** The owner's RTX 3060, the rig the archived goldens name (for example
`docs/archive/GUI-P5A-RENDER-PLAN.md`). Per NVIDIA's spec pages [P]:
- RTX 3060: 3584 CUDA cores at 1.78 GHz boost, 192-bit GDDR6.
- RTX 3070: 5888 cores at 1.73 GHz, 256-bit GDDR6.

That gives peak FP32 of 3584 × 1.78 × 2 = **12.8 TFLOPS** for the 3060 against 20.4 for the 3070. So a
compute-bound number measured on a 3070 scales by about **1.6×** to the 3060 [E].

**Memory bandwidth.** The RTX 3060 has 192 bit × 15 GT/s = **360 GB/s**; the RTX 3070 has
256 bit × 14 GT/s = **448 GB/s** [S 45]. NVIDIA's pages as fetched omitted the data rate. So a
bandwidth-bound number scales by **448 / 360 ≈ 1.24×** [E]. Pass 1 said 1.33× from bus width alone,
which ignored the 3060's faster memory.

**Instruction rate.** A single-FP32-op-per-lane rate is 3584 × 1.78 GHz ≈ **6.4 T lane-instructions/s**
[E]. Use it, not the FMA-doubled 12.8 TFLOPS, to convert an instruction count into time.

**Occupancy limits (compute capability 8.6, the 3060's):** 48 resident warps per SM, 64 K 32-bit
registers per SM, at most 255 registers per thread [P 46]. `docs/PARTICLES-PLAN.md` derives
warps/SM from a driver-reported register count with these limits and the 256-register-per-warp
allocation unit, and measured that **+2 registers crossed an allocation step** on the particle sim
(`docs/PARTICLES-PLAN.md:1377`).

---

## 0. The reading in six lines

1. **The tree cannot animate a material through its GPU table today** [T].
   - `MaterialTable::flush_if_dirty` has no production caller.
   - The staging ring it would fill is copied by no pass.
   - Every shader that reads `Materials[id]` therefore sees the boot values forever.
   - The one live route is a set of per-instance lanes that the gather re-copies from
     `Assets<Material>` every frame. They carry `base_color`, plus metallic and roughness on
     textured paths, and nothing else (§1.2).
   - The same path has two live defects, traced but not reproduced (§1.11): materials minted into
     the table's headroom stay all-zero on the GPU, and the per-instance material ring goes stale when
     its upload flag falls.
2. **Shipped engines split dynamic material data into three frequency classes.**
   - Global: UE Material Parameter Collections, Unity `_Time`, Godot `TIME`.
   - Per material: material instances.
   - Per instance: UE Custom Primitive Data, Unity Entities Graphics `[MaterialProperty]`, Godot
     instance uniforms, Bevy `MeshTag`.
   - A material instance per object is the documented anti-pattern in both UE and Unity (§2.1, §2.2).
3. **A visibility buffer shades N material programs by classifying pixels and issuing one
   indirect dispatch per program** (Hable; Nanite from UE 5.4).
   - The measured costs are a fixed classify tax (0.32–0.34 ms at 1080p on an RTX 3070 [P]), empty
     bins (81–92% empty in Epic's captures [S]) and PSO counts.
   - A GPU-chosen pipeline exists in Vulkan only through `VK_EXT_device_generated_commands`.
     NVIDIA ships it (Windows 553.19 / Linux 550.40.75 [P]); RADV ships it from Mesa 24.3 [S]; AMD's
     Windows driver was not verified (§2.9).
4. **Most effects in scope have no per-pixel input.** They are functions of time plus a per-material
   or per-instance parameter: pulsing emissive, colour gradients over time, blink and flicker, UV
   scroll offset, flipbook frame, hit flash, tint, wetness level.
   - Per pixel: fresnel rim, triplanar or procedural noise, dissolve noise, puddle masks.
   - Per vertex: wind and vertex animation textures (§2.12).
5. **The tree has already paid for much of the VB material machinery.**
   - The classify chain is live (`vb_use_classified`).
   - `vkCmdDispatchIndirect` is loaded and used.
   - Spec constants ship for compute pipelines.
   - The eDSL already generates the analytic UV derivatives Hable's scheme needs.

   It has **not** paid for:
   - an RHI-level indirect dispatch;
   - spec constants on graphics pipelines;
   - texture sampling or derivative propagation in the eDSL;
   - a GPU time source;
   - a masked raster (§1).
6. **Vertex animation is the most invasive class.** It reaches every raster vertex shader
   (shadows included), the VB shade's vertex re-fetch, meshlet and HZB cull bounds, motion vectors
   and HWRT acceleration structures.

---

## 1. What the tree holds today [T]

### 1.1 The material data path: CPU authority → GPU mirror → readers

| Fact | Where | Consequence |
|---|---|---|
| **The CPU authority.** `Assets<Material>` holds `Material { gpu: MaterialGpu, textures: MaterialTextures }`. `MaterialGpu` is `#[repr(C, align(16))]`, 48 B, three `vec4` lanes: `base_color` (`w` = alpha/cutoff), `mrr` = `[metallic, roughness, reflectance, bitcast(flags)]`, `emissive` (`w` unused). | `crates/boyko_render/src/material.rs:51-70`, size assert `:83` | The record is frozen. Its free space is `emissive.w`, the unread `base_color.w`, and flag bits 1..31. |
| **Only one flag bit is used.** `MATERIAL_FLAG_TEXTURED = 1` is re-derived at the upload boundary from `textures.any()`. `Material::new` passes `flags` through verbatim. | `crates/boyko_render/src/material.rs:121`, `Material::new` `:209`, `MaterialTable::seed_rows` | A bare `flags: 1` claims textures the material does not have. The host re-derives the bit for the GPU; a CPU reader of `gpu.mrr[3]` before upload is misled. |
| **Edits bump a whole-table generation.** `Assets::get_mut` bumps `dirty_gen` on every call that resolves to a live row (`add` does not; `fill` does). | `crates/boyko_ecs/src/ecs/core/asset/assets.rs:497-502` | Dirtiness is per table, not per row. |
| **The `dirty` bitmap is not an edit set.** `Assets` does carry a `dirty: LiveBitmap`, but it is streaming-lifetime plumbing: it is set on free and retire (`assets.rs:575`), and the struct doc says it "lands inert … (F2+ wires the readers)". | `assets.rs:189`, `:207` | No per-row "value edited" set exists. The `LiveBitmap` type such a set would use does. |
| **`flush_if_dirty` has no production caller.** Its only mention outside `material_table.rs` is a comment in `crates/boyko_app/tests/asset_streaming_f7_grow_headless.rs`. Its own doc says that at rung A1 it "never actually runs". | `crates/boyko_render/src/material_table.rs:255` | Tier 1 animation cannot reach the SSBO. |
| **The staging ring is read by nothing.** Per the type's own words it is read by nothing "until a staging→table GPU copy is wired" (section "O3, deferred"). | `material_table.rs:377-386` | Even a caller would fill buffers nobody copies. The same doc flags that staging must adopt per-slot fenced growth when that copy lands. |
| **The table and the staging ring are host-visible.** Both are created `MemoryLocation::HostVisibleCoherent`, at boot and on grow. The allocator takes the **first** memory type with `HOST_VISIBLE|HOST_COHERENT` (`select_memory_type`, `crates/boyko_rhi_vulkan/src/memory.rs`). On common NVIDIA type orders that is system memory, not the 256 MB BAR heap. **Inference, not measured on the rig.** | `material_table.rs:179, 199, 425, 447` | Every lit producer's per-pixel material fetch may be crossing PCIe on a cache miss. |
| **There is a hard row cap.** `MAX_MATERIAL_ROWS = 1 << 16`, because `MaterialId` is 16-bit. | `crates/boyko_render/src/material_table.rs:65` | Worst-case whole table: 65,536 × 48 B = **3.0 MiB**. |
| **The light table is the working precedent.** A per-FIF host staging slot is written only when its generation lags; a `light_upload` graph pass records `cmd_copy_buffer(light_staging → light_table)` into a device-local table on dirty frames only. It is recorded identically in `record_vb` (`crates/boyko_rhi_vulkan/src/present/passes/vb.rs:1107-1125`), `gbuffer.rs` and `forward.rs`; the host write lives in `crates/boyko_app/src/runner.rs` (`light_upload_due`, `upload_light_table`). | as cited | This is exactly the missing material copy, already built once. |
| **Eight shader sources bind and index `Materials[...]`:** `deferred_pbr`, `forward_opaque.fs`, `sdf_forward_march`, `sdf_gbuffer_composite`, `vb_geo`, `vb_resolve`, `vb_shade`, `vb_shade_split`. Pass 1 said eleven: `forward_opaque.vs` names the table only in comments, `gbuffer_mrt.fs` does not bind it, and `sdf_probe_update` does not bind it (next row). | `crates/boyko_rhi_vulkan/shaders/` (a grep for the declaration and the index, re-run in pass 2) | This is the blast radius of any `MaterialGpu` layout change. |
| **SDF-DDGI never sees a material value.** The probe update bounces a constant, `GI_BOUNCE_ALBEDO`, because "the update bind-set carries no material table". | `crates/boyko_rhi_vulkan/shaders/sdf_probe_update.comp.hlsl:164` | No material edit, static or animated, changes the indirect light. An animated emissive does not light its surroundings through DDGI. |
| **The per-instance lanes are denormalized material copies.** `gather_mesh_draws` (two `cfg(hwrt)` twins, `crates/boyko_render/src/mesh_draw.rs:1300` and `:1518`) takes `Res<Assets<Material>>` and fills two lanes. `PerInstanceMaterial { base_color, id, _pad: [u32; 3] }` is 32 B with **12 B spare** (`mesh_draw.rs:122-133`). `PerInstanceMaterialTex` is 48 B with no spare: `base_color`, `material_id`, five bindless slots, `metallic`, `roughness`. | `mesh_draw.rs` | These lanes are the only path that already carries live material values. |
| **The per-instance lanes upload only when needed.** The `PerInstanceMaterial` ring is uploaded **only on an `any_non_default_material` frame** (Principle 1). | `runner.rs:1786-1793` | This is the existing precedent for a structurally zero-cost per-instance lane. Its readers are not all gated the same way; that is a live defect (§1.11). |
| **The VB instance ring uploads every VB frame.** `upload_vb_instance_rows` runs on every frame whose boot-resolved path is `VisibilityBuffer`, with no content gate. `sync_vb_instance_ring` packs each `VbInstanceRow` from parallel scattered lanes (`ring`, `mesh_ids`, `inst_flags`). | `runner.rs` step 5b′; `crates/boyko_render/src/mesh_draw.rs:519` | A new VB per-instance value is one more parallel lane, following the `inst_flags` precedent, and never goes stale. |
| **Capabilities are read without filtering.** The gather reads the occlusion capability through `Option<&OcclusionCulling>` because a filtering `With`/`Enabled` "would silently RENUMBER the ring". | `mesh_draw.rs:1307-1313` | This is the in-tree rule for how a per-instance override component must be read. |

### 1.2 Where a runtime edit lands today

An edit via `Assets::get_mut` after boot reaches:

| Field | VB flat (`vb_resolve`, flat `vb_shade`) | VB textured (`vb_shade_tex*`, `vb_shade_split_tex*`) | Deferred (`gbuffer_mrt` raster + `deferred_pbr`) | Forward / F+ (`forward_opaque`) | SDF pixels (resolve / composite / forward march) |
|---|---|---|---|---|---|
| `base_color` | **stale**: `Materials[pm.id]` (`vb_resolve.comp.hlsl:269-270`) | **live** via `pmt.base_color` (`vb_shade.comp.hlsl:369`) | **live** via the PM/tex lane, only when a PM/tex raster variant is selected | **stale**: `Materials[input.mat_id]` | **stale** |
| metallic / roughness | stale | live via `pmt` (texture fallback scalars) | flat material: stale (`deferred_pbr` reads `Materials[mat_id]`); textured: live via `gPbr` | stale | stale |
| reflectance | stale | stale (`mt = Materials[pmt.material_id]`, `vb_shade.comp.hlsl:357`) | stale | stale | stale |
| emissive | stale | stale | stale (`deferred_pbr.hlsl:839`, `:851`) | stale | stale |

So a **pulsing emissive fails on every path**, and a colour change works on some paths and not
others. The owner memo of 2026-08-27 says Tier 1 "WORKS TODAY"; that statement is false at
`6394bc5e` (§1.10).

### 1.3 Time sources

| Source | What it is | Reaches a surface shader? |
|---|---|---|
| `Time` (`crates/boyko_ecs/src/ecs/core/time/time.rs:38`) | The kernel's virtual frame clock. Advanced once per frame. Hitch-clamped (250 ms), pausable, scalable, with real delta and real elapsed carried alongside. On the default path the delta is integer-nanosecond exact. | No |
| `FixedTime`, `ParticleClock` (`crates/boyko_render/src/particle_clock.rs:72`) | Fixed-rate clocks. Particles own theirs deliberately, so they do not force a `Fixed` schedule into existence. | No (particles use their own push constant) |
| `UiClock` (`crates/boyko_ui/src/animation.rs`) | Once-per-frame clamped real and virtual deltas. Clock policy AD9: tweens use the real clock by default and the virtual clock per row; flipbooks use the virtual clock. | UI only |

**The shader camera block is not `ViewUniform`.** Every lit producer's `cbuffer Camera` is the
80-byte `CompositePushConstants` (`crates/boyko_rhi_vulkan/src/compute.rs:3310-3326`):
- `count`, `img_w`, `img_h`, `camera_mode`, `cam_eye`, `cam_forward` (`w` = `tan(fovY/2)`),
  `cam_right` (`w` = aspect), `cam_up`;
- **`cam_eye.w` and `cam_up.w` are unused.**

`ViewUniform` (`crates/boyko_scene/src/camera.rs:253-267`) has four free `.w` lanes, but it is not the
block the shading tails bind. The upstream report named it as the time carrier; that is the wrong
struct.

The raster pushes are a different struct again: `gbuffer_mrt.vs` uses `pc.cam_eye.w` as the camera
mode, and `punctual_depth` uses it as `inv_range`.

### 1.4 Indirect commands and the RHI seam

| Item | State at `6394bc5e` |
|---|---|
| `vkCmdDispatchIndirect` | **Loaded**: `DeviceFns::cmd_dispatch_indirect` (`crates/boyko_rhi_vulkan/src/device.rs:597-599`), Vulkan 1.0 core. **Used** by particles, whose own comment calls it "the FIRST production consumer" (`present/passes/particles.rs:468`). |
| `vkCmdDrawIndexedIndirect` | Loaded (`crates/boyko_rhi_vulkan/src/device.rs:669-673`); used by VB (`vb.rs` batch draws) and particles. |
| `vkCmdDrawIndexedIndirectCount` | **Deliberately not loaded.** It needs `drawIndirectCount` in a `VkPhysicalDeviceVulkan12Features` the device never chains (`crates/boyko_rhi_vulkan/src/device.rs:669-673`). |
| `RhiCommandEncoder::dispatch_indirect` | A `#[cold] #[inline(never)]` **no-op default** (`crates/boyko_rhi/src/encoder.rs:398-410`); no backend overrides it. Passes that use indirect dispatch call the raw `DeviceFns` directly. |
| Async compute | One graphics+compute queue (`device.rs` module doc). No second queue. |
| `VK_EXT_device_generated_commands`, `VK_AMDX_shader_enqueue`, mesh shaders | Absent from the FFI. |

**Stale statements.**
- `vb.rs:2790-2791` says neither `vkCmdDispatchIndirect` nor the Count variant "is in this device's
  fn table". The first half is false; the second is true.
- `docs/VB-P2-CLASSIFICATION-PLAN.md:37` (D2) says "The FFI lacks indirect dispatch". That was true
  when written and is false now.

### 1.5 Visibility-buffer classification is live, not dark

`docs/VB-P2-CLASSIFICATION-PLAN.md` describes P2a as "dark infra, unwired", and the owner memo still
says so. The tree is at P2c:
- `crates/boyko_app/src/gpu_scene/mod.rs:6498` sets
  `vb_use_classified = vb_force_classified || vb_tex_active_this_frame`.
- `vb.rs` records `vb_classify_fill/count/scan/scatter` and then `vb_shade` when it holds.

The classified shade reads `mat = group_to_mat[g]` per 64-thread group, so **`mat_id` is
wave-uniform by construction** (`crates/boyko_rhi_vulkan/shaders/vb_classify_common.hlsli`).

`gClassify` is sized per frame in flight (`FRAMES_IN_FLIGHT = 2`,
`crates/boyko_rhi_vulkan/src/present/mod.rs:92`) by the header's formula
`words = 5·MAX + G + w·h`, with `MAX = 65,536` and `G = ceil(w·h/64)`:

| Extent | Words | Bytes per FIF | Both FIF |
|---|---|---|---|
| 1920×1080 | 327,680 + 32,400 + 2,073,600 = 2,433,680 | **9.73 MB** | 19.5 MB |
| 2560×1440 | 327,680 + 57,600 + 3,686,400 = 4,071,680 | **16.3 MB** | 32.6 MB |
| 3840×2160 | 327,680 + 129,600 + 8,294,400 = 8,751,680 | **35.0 MB** | 70.0 MB |

[E, arithmetic from the formula.] The plan quotes "~8.4 MB/FIF @1080p". It predates the `G + MAX`
reservation for `group_to_mat`.

**The tree already pays the classification tax on every textured VB frame.** That changes how much
Tier 3 adds on top (design doc F12).

### 1.6 Variants, pipelines, spec constants

- **114 `.spv` files** in `crates/boyko_rhi_vulkan/shaders/`. The lit producers among them:
  - **10 VB tails:** `vb_resolve{,_froxel}`, `vb_shade{,_froxel,_tex,_tex_froxel}`,
    `vb_shade_split{,_hwrt,_tex,_tex_hwrt}`;
  - **6 deferred resolves:** `deferred_pbr{,_wrap,_hwrt,_hwrt_vis,_hwrt_denoised,_hwrt_vis_mv}`;
  - **10 G-buffer raster modules** (five `vs`/`fs` pairs: base, `mv`, `mvpm`, `pm`, `tex`);
  - **3 Forward modules:** `forward_opaque.{vs,fs}`, `forward_opaque_froxel.fs`;
  - **4 SDF forward-march variants** and `sdf_gbuffer_composite`.

  Of the six deferred resolves, **four light a pixel**: `deferred_pbr{,_wrap,_hwrt,_hwrt_denoised}`.
  `_hwrt_vis` and `_hwrt_vis_mv` write shadow visibility and return before lighting (their DXC recipe
  comments in `deferred_pbr.hlsl`). So the sites where a lit pixel's `emissive` is formed are
  10 VB + 4 deferred + 2 Forward fragment + 4 SDF march = **20**. Of those, **18 are compute** and 2 are
  raster (`forward_opaque{,_froxel}.fs`).
- A `-D` axis on the shading tails multiplies across the existing axes (froxel × tex × split ×
  hwrt), not only across VB.
- **Spec constants ship, but for compute only.**
  - `GI_MAX_IT` on `sdf_probe_update` (`crates/boyko_rhi_vulkan/src/compute.rs:692-699`) and `SHADOW_RAY_COUNT` on the hwrt
    deferred resolve are production spec constants.
  - `ComputePipelineDesc::spec_constants` exists (`crates/boyko_rhi/src/descriptor.rs:262-267`).
  - **`GraphicsPipelineDesc` has no such field** (`crates/boyko_rhi/src/descriptor.rs:352`), so a raster permutation cannot
    become a spec constant without an RHI change.
  - The `spec_constant_smoke` shader is a separate, feature-gated test.
  - The manifest's "SHIPPED" (`docs/SHADER-VARIANT-MANIFEST.md:10`) is correct. The upstream
    report's "contradictory" reading came from conflating the two.
- **A runtime gate instead of a `-D` axis has a precedent.** `docs/RENDER-PARITY-PLAN.md` §3.6
  chose a "runtime uniform gate, one-time re-pin" over `-D SDF_SHADOW`. The binding and a
  never-taken uniform branch are always compiled in. Image goldens stay authoritative; the `.spv`
  bytes are re-blessed once.

### 1.7 The shader eDSL

- **Structure.** One generic Rust body per leaf, instantiated over `f32` (the host oracle) and
  `Emit(u32)` (a thread-local SSA recorder the HLSL printer walks). The `emit` feature is off by
  default.
- **Ops include:** arithmetic, `Min/Max/Clamp01/Lerp/Abs/Sqrt/Sin/Cos/Acos`, `Select*`, comparisons,
  integer and bit ops, `Vec2/Vec3/Vec4` params, `Vec2Frac`, `Vec2Lerp`, `Vec2Dot`, `Vec3Dot`,
  buffer loads, and one emit-only `Texture3D` resource node for the SDF atlas
  (`crates/boyko_shaderdsl/src/emit/mod.rs`).
- **What it cannot express.** Its own statement: "no atomics, no `groupshared`, no stores and no
  texture sampling" (`crates/boyko_shaderdsl/src/bin/emit_particles.rs:34`). So every skeleton
  (bindings, sampling, `discard`, stores) is hand-written or generator-owned HLSL wrapped around
  generated leaves.
- **`sin` is not host-stable.** The `f32` instance of `sin` is the host `f32::sin`
  (`crates/boyko_shaderdsl/src/interp.rs`), i.e. the platform libm, so it is **not** bit-stable
  across MSVC and glibc hosts.
- **The VB analytic derivatives already exist.** `vb_barycentric_grad`, `vb_interp` and `vb_uv_grad`
  are generated into `vb_geom_fetch.hlsli`, and the textured VB tails sample with those gradients.
  That is the derivative base Hable's scheme uses (§2.9). What is absent is derivative propagation
  through *procedural* values (a dual-number type) and a bindless `SampleGrad` resource node.
- **Generated leaves already sit in the raster.** `gbuffer_mrt.fs.hlsl` already carries two
  (`oct_encode`, `pack_material_id_ba`) under `gbuffer_mrt_edsl_sync`.

### 1.8 Motion vectors and `discard`

- **Per-object motion vectors are declared and deliberately not wired.**
  `MvSource::PerObject` (`crates/boyko_render/src/taa_config.rs:259`, variant at `:307`) records
  why:
  - the SDF-pixel motion-vector producer exists only under `hwrt`;
  - a mesh-only motion-vector buffer would regress SDF pixels to `Δuv ≡ 0`;
  - the ghosting it would fix was **not observed**, because the variance clamp rejects stale
    history.

  A `PrevInstanceModelCol` prev-instance ring exists under `feature = "hwrt"` only.
- **No opaque raster may discard.** `vb_raster.fs.hlsl:3` and `forward_opaque.fs.hlsl:7` both state
  "NO `discard`".
- **The masked rail is designed but not built.** The transparency design plans it: `-D MASKED`
  raster variants and `VbInstanceRow.flags` bit 1 = `VB_INST_FLAG_MASKED`, with a hashed threshold
  as an eDSL leaf ([`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) §1.2, §9).

### 1.9 Free-lane inventory

| Carrier | Free space | Read by |
|---|---|---|
| `MaterialGpu` | `emissive.w` (1 float), `base_color.w` (carried, never read; transparency claims it as cutoff), `flags` bits 1..31 | 8 shader sources (§1.1) |
| `PerInstanceMaterial` (32 B) | `_pad: [u32; 3]` = 12 B | the **6 flat** VB tails `vb_resolve{,_froxel}`, `vb_shade{,_froxel}`, `vb_shade_split{,_hwrt}` (`vb_layout0` binding 1 through `vb_set0` / `vb_set0_froxel`); the VB classify `count` and `scatter` (binding 1, `.id` only: `vb_classify_count.comp.hlsl:76`, `vb_classify_scatter.comp.hlsl:65`); the Deferred `pm` and `mvpm` raster; `forward_opaque.vs` (binding 1, `.id` only, `:157`). Uploaded only on flagged frames, read every frame by all but the Deferred raster (§1.11). |
| `PerInstanceMaterialTex` (48 B) | none. Its `material_id` word holds a 16-bit id, so its upper 16 bits are the only free bits. | the **4 textured** VB tails `vb_shade_tex{,_froxel}`, `vb_shade_split_tex{,_hwrt}`: the **same** binding 1, bound through a distinct set against the same `vb_layout0` object (`crates/boyko_rhi_vulkan/shaders/vb_shade.comp.hlsl:92-110`; `vb_set0_tex` and `vb_set0_tex_froxel`, `crates/boyko_rhi_vulkan/src/present/targets.rs:657`, `:687`). Also the Deferred `tex` raster. **A value placed in `PerInstanceMaterial` is therefore invisible to every textured VB frame.** Pass 1's "all 10" was wrong. |
| `VbInstanceRow` (64 B) | `_pad: [u32; 2]` at offset 56 (`crates/boyko_render/src/instance_model.rs:270`), plus `flags` bits 1..31 (bit 1 reserved by the transparency design) | every VB shading shader and the VB cull. `vb_geom_fetch` loads the whole row (`VbInstanceRow inst = gVbInstances[instance_id]`), and offset 56 sits in the same 16-B lane as `mesh_id`, so reading it costs no extra fetch. Uploaded unconditionally on every VB frame (§1.1). |
| `SdfEdit` (48 B) | `_pad: u32` only; `center.w` carries the material id (`crates/boyko_sdf_math/src/lib.rs:197-209`). **`params.w` is not free:** it is the capsule radius (`edit_view`, `lib.rs:732`, written by `SdfEdit::capsule`). The struct's own layout and field docs still say "w unused" (`lib.rs:125`, `:135`); pass 1 repeated that stale claim. | the SDF marcher |
| Deferred G-buffer | `gAlbedo.a`: written `1.0` by every raster variant (`gbuffer_mrt.fs.hlsl:235-240`), never read by the resolve (`deferred_pbr.hlsl:820` loads the texel and uses `.rgb`) | none |
| `CompositePushConstants` (80 B) | `cam_eye.w`, `cam_up.w` | none |

### 1.10 Corrections to earlier statements

| Statement | Source | What the tree says |
|---|---|---|
| Tier 1 "WORKS TODAY, zero new machinery" | owner memo 2026-08-27 (session memory, not in the tree) | False: no caller, no copy (§1.1–§1.2). |
| Tier 3 blocker 1: "`vkCmdDispatchIndirect` is NOT in the FFI" | owner memo; `VB-P2-CLASSIFICATION-PLAN.md:37` | Loaded and used (§1.4). The remaining gap is the RHI trait method. |
| VB-P2 is "dark infra, unwired" | owner memo; plan rung table | P2c is live (§1.5). |
| Spec-constant status is contradictory | upstream report | Shipped for compute; absent for graphics (§1.6). |
| The time carrier is `ViewUniform`'s free lanes | upstream report | Wrong struct: shaders bind `CompositePushConstants` (§1.3). |
| Base-colour edits reach the Deferred per-instance raster and the textured VB tails | upstream report | Correct, and it is also the *only* live route (§1.2). |
| `SdfEdit.params.w` is "unused" | `crates/boyko_sdf_math/src/lib.rs:125`, `:135` (struct docs); pass 1 of this survey | It is the capsule radius (`edit_view`, `lib.rs:732`) (§1.9). |

### 1.11 Two live defects on the material data path

Neither was reproduced. Both follow from the code by a short trace, and the design's DM1 carries a
red-first test for each.

**D-1: a material minted into the table's headroom never reaches the GPU.**
- `MaterialTable::grow_if_needed` returns early whenever `high_water` fits the current capacity
  (`crates/boyko_render/src/material_table.rs:404`).
- A grow rounds capacity up to the next power of two (`:407`) and seeds only the rows that are
  `Loaded` at that moment.
- Nothing else writes the table after boot. `flush_if_dirty` has no caller (§1.1), and `add` does not
  bump `dirty_gen`.
- So a material minted, or streamed in by `fill`, into the headroom `[need, capacity)` after a grow
  keeps an all-zero row: black base colour, zero roughness. If no later grow re-seeds it, that is
  forever.
- The in-tree F7 grow test (`crates/boyko_app/tests/asset_streaming_f7_grow_headless.rs`) mints one
  material per frame from capacity 1. It checks buffer identity, not pixels, so it cannot see this.

**D-2: the `PerInstanceMaterial` ring goes stale when its flag falls.**
- The ring is uploaded only on an `any_non_default_material` frame (`runner.rs:1788`).
- Its readers on VB and Forward read it every frame: `vb_resolve.comp.hlsl:269`, the classify
  (`vb_classify_count.comp.hlsl:76`) and `forward_opaque.vs.hlsl:157`. The Deferred `pm` raster is
  the only reader that is deselected with the flag (`crates/boyko_app/src/gpu_scene/mod.rs:7066-7068`).
- When the last non-default-material instance goes away, the flag falls. Both frame-in-flight slots
  keep their last bytes. The gather renumbers the ring every frame, so the unconditional readers
  fetch another instance's old `id` at each index.
- Consequence: wrong materials on VB and Forward after the last non-default material instance
  despawns, until the flag rises again.
- A grown ring is zero-filled (`gpu_scene/mod.rs:5885`), so a ring that was never written is
  correct. Only a ring that was written and then un-flagged is wrong.

---

## 2. What shipped engines do

### 2.1 Unreal Engine

**Material Parameter Collections: global parameters.**
- Limits: up to 1,024 scalar and 1,024 vector parameters per collection; a material may reference at
  most two collections [P 1].
- Changing a collection's parameter count recompiles every referencing material [P 1].
- Epic calls updating a collection much more efficient than setting parameters on many material
  instances [P 1].

**The `Time` node** has "Ignore Pause" and "Period" options. On mobile, Period's wrap is computed on
the CPU at full precision while the GPU runs at half precision, with a warning about periods longer
than a minute [P 4].

**Material instances.**
- A constant instance cannot change at runtime. A dynamic instance (MID) can [P 3].
- A static switch compiles one shader per combination used by instances [P 3].
- A practitioner reports that each MID is its own draw with no batching ("50 traffic lights → 50
  draw calls") and, under Nanite, its own shading bin. Each MID is also a UObject, which can cause a
  garbage-collection hitch [S 7].

**Custom Primitive Data: per-primitive floats.**
- Indexed by number, not by name.
- The docs say 32 floats and note it may become a project setting. The practitioner counts nine
  `float4`s, i.e. 36 [P 2 / S 7].
- Epic's own test: 25 spheres drew 94 mesh draws without dynamic instancing and 46 with it [P 2].
- It cannot change textures [S 7].
- Instanced static meshes carry the same idea per instance: *Per Instance Custom Data* passes data
  "without generating a new dynamic material instance per mesh" [P 44].

**Per-instance desynchronisation.** `PerInstanceRandom` outputs "a different random float value per
Static Mesh instance" [P 4]. Combined in the graph with `Time`, it is how instances of one material
stop animating in lockstep. The evaluation is on the GPU, per pixel, from a per-instance seed.

**Nanite material shading.**
- *UE 5.0* (capture analysis [S 23]):
  - the material id is written as depth into a "material depth" target;
  - a 20×12 Material Range texture (one texel per 64×64 region) lets the vertex shader cull tiles;
  - each material is drawn as one full-screen quad cut into 240 tiles, with a depth-EQUAL test;
  - ids are 14-bit (16,384 materials).
- *UE 5.4* (Wihlidal, GDC 2024, via notes [S 21] and a forum post quoting the slides [S 22]):
  - count per 2×2 quad → reserve offsets and indirect arguments → Morton scatter → one indirect
    dispatch per shading bin;
  - 3,075 of 3,779 bins were empty (81%) and compaction saved "nearly 1 ms"; City Sample had 3,675
    of 4,015 empty (92%);
  - `SetPredication` was tried and was too costly per PSO/descriptor change, so work graphs were used
    (D3D12 only);
  - the naive compute port was *slower* than 5.0 (already recorded in
    [`MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`](../MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md)).
- *"Programmable raster" bins* cover masked, two-sided, pixel-depth-offset, world-position-offset
  and custom-UV materials, each in hardware and software rasteriser variants [S 21].

**World-position offset (WPO).**
- In Nanite, WPO meshes need clamped displacement and cluster-bound handling [P 6].
- WPO writes no velocity unless `r.Velocity.EnableVertexDeformation` is on [P 37].

**PSO cost.** Epic's PSO precaching page reports 5–10 ms compile hitches with one at 117 ms, and
precache memory of "hundreds of megabytes or even gigabytes" [P 5].

### 2.2 Unity

**Built-in time values** [P 11]:
- `_Time = (t/20, t, 2t, 3t)`;
- `_SinTime` and `_CosTime` over `(t/8, t/4, t/2, t)`;
- `unity_DeltaTime = (dt, 1/dt, smoothDt, 1/smoothDt)`.

**The SRP Batcher** keeps a persistent `UnityPerMaterial` constant buffer in GPU memory per
material. Engine properties go through "a dedicated code path" into a large GPU buffer. Editing a
material re-uploads that material's buffer. A `MaterialPropertyBlock` makes the renderer
SRP-Batcher-incompatible [P 8].

**Entities Graphics is the ECS-native precedent.**
- A component such as
  `[MaterialProperty("_Color")] struct MyOwnColor : IComponentData { float4 Value; }` overrides a
  shader property declared "Hybrid Per Instance" [P 9].
- When the component is absent, the property is **zero-filled** [P 9].
- Under DOTS instancing, each property gets a 32-bit metadata value at `AddBatch`, and data is read
  from `ByteAddressBuffer unity_DOTSInstanceData`. The first 64 bytes should be zero, because
  metadata defaults to 0 [P 10].
- The docs do not state upload frequency or how changes are detected.

**`#pragma dynamic_branch`** turns keywords into uniform ints. Unity recommends it on fast GPUs
without asymmetric branches. Variants cost build time, file size, runtime memory and load time
[P 12].

### 2.3 Godot

- **`TIME`** rolls over at `rendering/limits/time/time_rollover_secs` (default 3,600 s). It follows
  `time_scale` but not pause [P 14].
- **`instance uniform`:** in practice at most 16 per shader, scalar and vector types only (no
  textures or arrays), set per `GeometryInstance3D` [P 13].
- **MultiMesh `INSTANCE_CUSTOM`** gives 4 floats per instance [P 15].
- **Visual shaders** are converted to shader text [P 15].

### 2.4 Bevy (0.16)

- **Materials.** `AsBindGroup` materials; `ExtendedMaterial<B, E>` merges base and extension bindings
  and may replace the vertex or fragment shader; `specialize()` builds a pipeline per material type
  [P 17].
- **Bindless.** Custom WGSL materials stay CPU-driven unless they use `#[bindless]`.
- **Per-instance data.** `MeshTag` is one `u32` per mesh entity, read by `get_tag(instance_index)`,
  to index a user buffer.
- **Speed claim.** About 3× over 0.15 on the Caldera hotel scene [P 16, project claim].

### 2.5 Frostbite (GDC 2007 deck, primary)

Andersson & Tatarchuk, *Frostbite Rendering Architecture and Real-time Procedural Shading &
Texturing Techniques* [P 38]:
- **Graph shaders.** Surface shaders are directed acyclic graphs with a root node that determines
  output colour and opacity. Nodes have inputs (connected) and properties (fixed values).
- **Reusable subgraphs.** "Instance shaders" are reusable graph networks used like functions. One of
  them, *StandardRoot*, is the base for about 90% of the shaders.
- **Permutations are built offline.** An offline pre-processing system collects the "wanted state
  combinations" that runtime systems report and generates the shading solutions ahead of time.
- **The motivating problem** was a previous project with about 500 hand-written custom shaders
  (ToyShop).

The deck does not describe runtime parameter animation or per-instance data.

### 2.6 Decima — not covered

The upstream session never searched it. This pass reached:
- Guerrilla's publications feed (no materials or shader talk listed);
- the *Creating a Tools Pipeline for Horizon Zero Dawn* abstract (no material-system detail).

**No claim about Decima appears in either document.** Closing the gap needs the slides of a
Guerrilla material-system talk. None was located without search.

### 2.7 RedLynx / Ubisoft (Aaltonen & Haar, SIGGRAPH 2015, primary)

*GPU-Driven Rendering Pipelines*, Advances course [P 39]; the text was extracted from the deck:
- **Everything is virtual-textured.** A 256k² virtual atlas with 128² pages and an 8k² page cache (a
  5-slice array: albedo, specular, roughness, normal, …). All texture data is reachable through one
  binding, so draws need no batching by texture.
- **Virtual deferred texturing.** The G-buffer stores 16+16-bit UV into the page cache plus a 32-bit
  tangent-frame quaternion: "64 bits. Full fill rate. No MRT." The **material id and mip are
  implicit from the page**. Gradients are reconstructed from screen-space UV distance.
- **Material blends are cached.** Complex material blends and decals are baked into the page cache
  and reused over hundreds of frames.
- **One draw per viewport.** A viewport is a single draw call (×2). **Dynamic branching selects the
  vertex animation type, at a reported +2% cost.**
- **The MSAA trick.** G-buffer time 3.03 → 2.06 ms (−32%) on Xbox One at 1080p.

This is the "one shader for every material" end of the design space. Its price is a virtual-texturing
system, and time-varying procedural results are not cacheable in the page cache.

### 2.8 id Tech (Doom 2016 / Eternal)

- Pettineo cites about 100 shaders and about 350 PSOs for the Doom games [S 18].
- A study of Doom Eternal reports that it "supposedly has about ~500 pipeline states", with almost
  all materials combined into very few uber shaders [S 19].

### 2.9 Shading N material programs over a visibility buffer

**Burns & Hunt, JCGT 2013 [P 25]** is the origin of the visibility buffer. The URL resolves, but the
PDF text was not extracted in either pass.

**Stachowiak 2015 [S 24]:** "indirect dispatch of lots of unique compute shaders", decoupling
geometry from materials and avoiding uber-shaders.

**Hable, *Visibility Buffer Rendering with Material Graphs* [P 20; re-fetched and verified by this
pass]:**
- *Rig:* RTX 3070 at 1920×1088 (the framebuffer is rounded to a multiple of 16).
- *Materials:* two texture sets blended with three octaves of Perlin noise, plus a third layer.
- *Pipeline:* count per material → prefix sum → reorder pixel XY → **one indirect compute dispatch
  per material** writing G-buffer data.
- *Derivatives* are analytic, from barycentric derivatives via the chain rule, and textures sample
  with `SampleGrad`.

| Triangle size | Material pass: deferred | Material pass: visibility | Lighting: deferred / visibility | Fixed classify cost ("VisUtil") |
|---|---|---|---|---|
| large quads | 1.06 ms | 1.06 ms | 0.730 / 0.762 ms | 0.322 ms |
| ~8–10 px | 2.95 ms | 1.65 ms | 0.764 / 0.818 ms | ≈0.34 ms |
| 1 px | 4.64 ms | 2.01 ms | 0.792 / 0.836 ms | ≈0.34 ms |

The article states "0.34ms of fixed cost". It does not discuss register pressure or empty-dispatch
cost.

**Scaled to the RTX 3060 [E]:**
- 0.34 ms × 1.24–1.6 (bandwidth-bound to compute-bound; see the reference-GPU note) →
  **0.42–0.54 ms at 1080p**;
- linear in pixels: **0.75–0.97 ms at 1440p** (×1.78) and **1.7–2.2 ms at 4K** (×4).

Pass 1 used 1.33 for the lower bound, which gave 0.45 ms at 1080p.

**GPU-chosen pipelines.**
- *AMD work-graph mesh nodes:* "ExecuteIndirect is 1.64x slower … on average" [V 28; RX 7900 XTX,
  pre-release driver].
- *Vulkan work graphs* exist only as `VK_AMDX_shader_enqueue`: provisional, "should not be used in
  production", no validation-layer support [P 29].
- *`VK_EXT_device_generated_commands`* (Vulkan 1.3.296) [P 30]:
  - "indirect execution sets" switch between pipelines that share one `VkPipelineLayout`, per
    sequence (an optional feature);
  - a sequence has exactly one dispatch token, placed last;
  - some implementations need a preprocess buffer.
- *Driver support:*
  - **NVIDIA:** Vulkan beta driver of 2024-09-26, **Windows 553.19 / Linux 550.40.75** [P 40,
    re-fetched].
  - **RADV:** GFX8+ from Mesa 24.3 [S 30].
  - **AMD Windows, Intel:** not verified (the gpuinfo coverage table did not render when fetched).

### 2.10 Uber-shader, permutations or spec constants

- **Rare branches still cost registers.** AMD: a single spike in one branch can make the whole
  shader need many VGPRs, even if the branch is never taken. Also: lower occupancy can help, but
  check that low-occupancy shaders are not spilling. On RX 7900 XTX, 1,536 VGPRs per SIMD at 120
  VGPRs gives 12 wave32 waves [P 26].
- **Pettineo's catalogue** [S 18]:
  - compile only what is used (Lone Echo 2 compiled about 5,000 pixel shaders and ran about
    900–1,000 PSOs);
  - spec constants cut offline compiles but not PSO count, and driver optimisation of them is
    "unpredictable";
  - uber-shaders with branching;
  - deferred or visibility decoupling;
  - offline linking and dynamic function calls, limited to compute and ray tracing on Vulkan and
    D3D12.
- **Divergent material ids in one wave.** Sawicki's waterfall loop (`WaveReadLaneFirst` plus a
  ballot per unique id) can hang (a TDR) on pixel-shader helper lanes. On Vulkan it may need
  `SPV_KHR_maximal_reconvergence` [S 27].
- **The tree already removes that divergence on one path.** The classified VB shade takes one
  material per 64-thread group (§1.5), and a 64-thread group is two whole NVIDIA warps. So a switch on
  any per-material key there (an uber-shader's program id) diverges only between waves, never inside
  one. Its remaining cost is AMD's: the worst program's registers are allocated for every program.
- **A specialization constant is folded, or not, by the driver.** Pettineo calls the optimisation
  "unpredictable" [S 18]. The tree can observe the outcome: `VK_KHR_pipeline_executable_properties`
  reports a pipeline's register count and ISA size, and `docs/PARTICLES-PLAN.md` already reads both
  from a headless probe.

### 2.11 Authoring: graph → code

- **UE material graphs and Godot visual shaders** compile to shader text [P 3, P 15].
- **MaterialX** has a generator per target language. A node implementation may be an inline
  expression, a function, a node graph or C++. The graph is topologically sorted, then emitted, and
  uniforms are published by name [P 32].
- **Slang** (He, Fatahalian, Foley, SIGGRAPH 2018) replaces preprocessor permutations with generics
  and interface constraints. The rearchitected renderer was simpler and "higher" in performance than
  its HLSL original [P 31, abstract].
- **Frostbite** built its permutations offline from reported state combinations (§2.5).

### 2.12 Effect by effect

| Effect | Shipped practice | Varies with |
|---|---|---|
| Pulsing or gradient emissive, blink, flicker | A material expression over global time (UE `Time`, Unity `_Time`, Godot `TIME`) | time × per-material params. **No per-pixel input.** |
| Scrolling UV | `uv + rate·t`; Unity's per-texture `_ST` (scale/offset) is the static form | the offset is per material; the `uv` it applies to is per pixel |
| Flipbook (explosions, fire) | Sub-UV frames. Lozar: cross-fade frames guided by an RG motion-vector texture; an 8×8 sheet (64 frames, about 2 s at 30 fps) stretched to more than 10× its perceived length; the motion texture must be uncompressed and can be much smaller (512² for a 4,096² base) [S 35] | frame index and blend weight are per material or per instance; the sampling is per pixel |
| Hit flash, per-instance tint | UE Custom Primitive Data, Unity `[MaterialProperty]`; a MID per object is the anti-pattern [P 2, P 9, S 7] | per instance |
| Out-of-phase flicker, staggered flipbooks | UE `PerInstanceRandom` × `Time` in the graph [P 4]; per-instance custom data [P 44] | per instance × time |
| Dissolve | Noise threshold + alpha test. Under Nanite this puts the mesh in a masked "programmable raster" bin [S 21] | per-instance progress; per-pixel noise; needs a masked raster path |
| Rain wetness | Lagarde: a global `WetLevel` in [0,1] rises and falls with rain; per-material `porosity` sets diffuse darkening, `factor = lerp(1, 0.2, (1−metalness)·porosity)`; gloss moves toward 1; puddles are painted in vertex colour and driven by `FloodLevel`; ripples come from a baked ripple texture. Measured on PS3/360 at 720p: ripple generation 0.14/0.15 ms, the whole rain system about 2.8 ms [S 34] | global × per-material (wetness); per pixel (puddles, ripples) |
| Fresnel rim | `pow(1 − N·V, k)` on emissive | per pixel (N·V) × per-material params |
| Triplanar or procedural noise | No primary source was retrieved (bgolus.medium.com returned 403). Derivation: three projections means three samples per map [E] | per pixel |
| Wind | Crysis (GPU Gems 3 ch. 16): per-vertex colours (R edge stiffness, G phase, B overall stiffness, A AO) plus a per-instance wind vector the CPU sums "similar to light sources" [P 33] | per vertex × per instance × time |
| Baked simulation | SideFX vertex animation textures: position and rotation per vertex per frame, 8-bit or 16-bit/HDR; soft, rigid, fluid and sprite modes [P 36] | per vertex × time |

### 2.13 Time precision in practice

- **Wrap or not:** UE wraps per use ("Period") [P 4]; Godot wraps globally every hour [P 14]; Unity
  exposes raw `t` [P 11].
- **The f32 arithmetic** [E]:
  - near t = 3,600 s one ulp is 2⁻¹² s ≈ 0.24 ms;
  - near t = 86,400 s (one day) it is 2⁻⁷ s ≈ 7.8 ms, about half a 60 Hz frame;
  - rounding to nearest errs by at most half an ulp, 3.9 ms. A 20 Hz term then carries a phase error
    up to 2π·20·0.0039 ≈ **0.49 rad**, still visibly wrong. Pass 1 used a full ulp and said 0.98 rad.
- **A wrapped time only stays continuous under a constraint** [E]. `t mod P` is continuous for
  `sin(2πft)` only if `f·P` is an integer, and for a UV scroll only if `rate·P` is a whole number of
  repeats.

### 2.14 Destiny (Bungie): TFX, one expression, two evaluators

*TFX: Destiny Shader System*, Tatarchuk & Tchou, GDC 2017 [P 43]:
- TFX language expressions and authored function expressions compile at import time to one bytecode
  representation, "interpreted on the CPU (or GPU) at runtime".
- So a shipped engine evaluates material expressions on the CPU from the same authored source that
  can also run on the GPU. That is the shape of the design's F4 (CPU drivers), and a precedent for
  its F14 (one program, a host evaluator and a GPU one).
- The difference from this tree: TFX interprets one bytecode on both sides. The eDSL instead compiles
  one generic Rust body into two instances, the `f32` host oracle and the HLSL printer (§1.7).

---

## 3. External numbers, with provenance

| Number | Value | Rig / condition | Class |
|---|---|---|---|
| VB classify fixed cost | 0.32–0.34 ms | RTX 3070, 1920×1088 | [P 20] |
| VB material pass vs deferred | 1.06/1.65/2.01 vs 1.06/2.95/4.64 ms | same | [P 20] |
| Nanite empty shading bins | 3,075 / 3,779 (81%); 3,675 / 4,015 (92%) | UE 5.4 captures | [S 21, S 22] |
| Bin compaction saving | "nearly 1 ms" | UE 5.4 | [S 21] |
| Empty-bin cost per bin | ≈ 1 ms / 3,075 ≈ **0.33 µs** | derived from the above; GPU unknown | [E] |
| UE PSO compile hitch | 5–10 ms, one at 117 ms | UE docs | [P 5] |
| Doom PSOs | ~350 (Doom), ~500 (Eternal) | | [S 18, S 19] |
| Lone Echo 2 | ~5,000 pixel shaders compiled, ~900–1,000 PSOs live | | [S 18] |
| ExecuteIndirect vs mesh nodes | 1.64× slower on average | RX 7900 XTX, pre-release driver | [V 28] |
| Vertex-animation-type branch | +2% | Xbox One era | [P 39] |
| Rain system | ~2.8 ms (ripples 0.14/0.15 ms) | PS3/360, 720p | [S 34] |
| Custom Primitive Data batching | 94 → 46 draws for 25 spheres | UE docs | [P 2] |
| RTX 3060 / 3070 FP32 | 12.8 / 20.4 TFLOPS | 3584 × 1.78 GHz × 2 / 5888 × 1.73 GHz × 2 | [P 41, P 42] → [E] |
| RTX 3060 / 3070 memory bandwidth | 360 / 448 GB/s (ratio 1.24) | 192 bit × 15 GT/s / 256 bit × 14 GT/s | [S 45] → [E] |
| CC 8.6 occupancy limits | 48 warps/SM, 64 K registers/SM, ≤ 255 per thread | the RTX 3060's compute capability | [P 46] |
| VB classify, scaled to the RTX 3060 | 0.42–0.54 / 0.75–0.97 / 1.7–2.2 ms | 1080p / 1440p / 4K | [P 20] → [E] |
| gClassify per FIF | 9.73 / 16.3 / 35.0 MB | 1080p / 1440p / 4K | [T] → [E] |
| Whole material table | 3.0 MiB at the 65,536-row cap | 48 B × 65,536 | [T] → [E] |

---

## 4. Pitfalls practitioners documented

1. **A material instance per object.**
   - In UE it kills batching and multiplies Nanite shading bins [S 7, P 2].
   - A Unity `MaterialPropertyBlock` breaks the SRP Batcher [P 8].
2. **Empty bins still cost a PSO bind.** 81–92% were empty in Epic's captures; predication was too
   costly; Epic had to engineer compaction [S 21].
3. **A rarely taken branch still allocates registers for its worst case** [P 26]. A static switch
   compiles one shader per used combination [P 3].
4. **PSO hitches.** 5–10 ms compiles and gigabytes of precache [P 5]. The tree's offline, committed
   `.spv` model avoids the compile, but not the pipeline-creation count.
5. **Time precision.** Half precision past about a minute on mobile [P 4]; Godot's hourly wrap
   [P 14]; f32 time loses more than a frame of resolution within a day [E, §2.13].
6. **Vertex animation silently breaks neighbours.**
   - Velocity: WPO writes none by default [P 37].
   - Cluster culling: displacement must be clamped [P 6].
   - In this tree, by derivation: the HZB fixed point, the static BLAS and the cull bounds.
7. **Scalarization loops** can TDR on helper lanes and need maximal reconvergence on Vulkan [S 27].
8. **Parameters addressed by a bare number** are mistyped silently [S 7]. The tree's `flags` key is
   the same hole (§1.1).
9. **Rain costs real frame time** (~2.8 ms for a PS3-era system) [S 34]. Wetness alone is cheap
   per-material math.

---

## 5. Gaps

| Gap | Why it matters | How to close it |
|---|---|---|
| Decima material system | A second AAA precedent for graph materials under a single-queue console budget | A Guerrilla talk's slides |
| AMD Windows and Intel support for `VK_EXT_device_generated_commands` | Decides whether DGC can ever be the default rather than a probed extra | gpuinfo coverage, AMD release notes |
| Wihlidal GDC 2024 slides, first-hand | The 81%/92% and "nearly 1 ms" figures are secondary | The GDC Vault PDF (too large to fetch in this pass) |
| Burns & Hunt 2013 text | Origin paper, cited from its abstract page only | The JCGT PDF |
| A primary triplanar-cost source | The three-samples-per-map figure is a derivation | A shipped-engine talk or docs page |
| Whether `HostVisibleCoherent` lands in system memory on the rig | Decides whether the device-local table is a perf change or only a protocol change | Log the chosen memory type's heap flags on the RTX 3060 |
| A primary source for the 3060/3070 memory data rate | The 1.24× bandwidth ratio rests on a secondary table | NVIDIA's full-spec pages or the GA106 datasheet |
| Whether the NVIDIA driver folds a `false` specialization constant out of a lit tail | Decides the design's F9 per site | The register probe (design F9, DM4 gate (c)) |
| The register count of every lit producer today | No tail's occupancy bucket is known, so no block's dark tax can be predicted | The same probe, run once over the committed `.spv` |

---

## 6. Sources

1. Epic, *Using Material Parameter Collections* — https://dev.epicgames.com/documentation/en-us/unreal-engine/using-material-parameter-collections-in-unreal-engine
2. Epic, *Storing Custom Data in Materials Per-Primitive* — https://dev.epicgames.com/documentation/unreal-engine/storing-custom-data-in-unreal-engine-materials-per-primitive
3. Epic, *Instanced Materials* — https://dev.epicgames.com/documentation/en-us/unreal-engine/instanced-materials-in-unreal-engine
4. Epic, *Constant Material Expressions* (Time node) — https://dev.epicgames.com/documentation/en-us/unreal-engine/constant-material-expressions-in-unreal-engine
5. Epic, *PSO Precaching* — https://dev.epicgames.com/documentation/en-us/unreal-engine/pso-precaching-for-unreal-engine
6. Epic, *Nanite Virtualized Geometry* — https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine
7. H. Thiessen, *Why you shouldn't use Dynamic Material Instances* — https://haukethiessen.com/why-you-shouldnt-use-dynamic-material-instances/
8. Unity, *SRP Batcher* — https://docs.unity3d.com/2022.3/Documentation/Manual/SRPBatcher.html
9. Unity, *Material overrides using C#* (Entities Graphics 1.2) — https://docs.unity3d.com/Packages/com.unity.entities.graphics@1.2/manual/material-overrides-code.html
10. Unity, *DOTS Instancing shaders* and best practice — https://docs.unity3d.com/2022.3/Documentation/Manual/dots-instancing-shaders.html ; https://docs.unity3d.com/6000.0/Documentation/Manual/dots-instancing-shaders-best-practice.html
11. Unity, *Built-in shader variables* — https://docs.unity3d.com/6000.3/Documentation/Manual/SL-UnityShaderVariables.html
12. Unity, *Choose a type of shader conditional* — https://docs.unity3d.com/6000.0/Documentation/Manual/shader-conditionals-choose-a-type.html
13. Godot, *Shading language* (instance uniforms) — https://docs.godotengine.org/en/stable/tutorials/shaders/shader_reference/shading_language.html
14. Godot PR #95381 (`TIME` rollover) — https://github.com/godotengine/godot/pull/95381
15. Godot, *Visual shaders*; *MultiMesh* — https://docs.godotengine.org/en/stable/tutorials/shaders/visual_shaders.html ; https://docs.godotengine.org/en/stable/classes/class_multimesh.html
16. Bevy 0.16 release notes — https://bevy.org/news/bevy-0-16/
17. Bevy `ExtendedMaterial` — https://docs.rs/bevy/latest/bevy/pbr/struct.ExtendedMaterial.html
18. M. Pettineo, *The Shader Permutation Problem*, parts 1–2 — https://therealmjp.github.io/posts/shader-permutations-part1/ ; https://therealmjp.github.io/posts/shader-permutations-part2/
19. S. Coenen, *Doom Eternal graphics study* — https://simoncoenen.com/blog/programming/graphics/DoomEternalStudy
20. J. Hable, *Visibility Buffer Rendering with Material Graphs* — https://filmicworlds.com/blog/visibility-buffer-rendering-with-material-graphs/
21. Notes on Wihlidal, *Nanite GPU-Driven Materials* (GDC 2024) — https://www.sctheblog.com/blog/nanite-materials-notes/ (primary slides: https://media.gdcvault.com/gdc2024/Slides/GDC+slide+presentations/Nanite+GPU+Driven+Materials.pdf, not fetched)
22. Unreal forum thread quoting the GDC 2024 slides — https://forums.unrealengine.com/t/why-cpu-needs-to-first-launch-all-possible-shading-bin-dispatches-even-after-culled-most-of-them/2525080
23. E. López, *A Macro View of Nanite* — https://www.elopezr.com/a-macro-view-of-nanite/
24. T. Stachowiak, *A deferred material rendering system* — http://h3.gd/a-deferred-material-rendering-system/
25. C. Burns, W. Hunt, *The Visibility Buffer*, JCGT 2(2), 2013 — https://jcgt.org/published/0002/02/04/
26. AMD GPUOpen, *Occupancy explained* — https://gpuopen.com/learn/occupancy-explained/
27. A. Sawicki, *A better way to scalarize a shader* — https://asawicki.info/news_1735_a_better_way_to_scalarize_a_shader
28. AMD GPUOpen, *GDC 2024: Work graphs and draw calls* — https://gpuopen.com/learn/gdc-2024-workgraphs-drawcalls/
29. Khronos, `VK_AMDX_shader_enqueue` — https://docs.vulkan.org/refpages/latest/refpages/source/VK_AMDX_shader_enqueue.html
30. Khronos, `VK_EXT_device_generated_commands` proposal; RADV notes — https://github.com/KhronosGroup/Vulkan-Docs/blob/main/proposals/VK_EXT_device_generated_commands.adoc ; https://rg3.name/202411181555.html ; https://www.phoronix.com/news/RADV-VK-EXT-DGC
31. Y. He, K. Fatahalian, T. Foley, *Slang*, SIGGRAPH 2018 — https://history.siggraph.org/learning/slang-language-mechanisms-for-extensible-real-time-shading-systems-by-he-fatahalian-and-foley/
32. MaterialX, *Shader Generation* — https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/documents/DeveloperGuide/ShaderGeneration.md
33. GPU Gems 3, ch. 16, *Vegetation Procedural Animation and Shading in Crysis* — https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis
34. S. Lagarde, *Water drop 2b* and *3b* — https://seblagarde.wordpress.com/2013/01/03/water-drop-2b-dynamic-rain-and-its-effects/ ; https://seblagarde.wordpress.com/2013/04/14/water-drop-3b-physically-based-wet-surfaces/
35. K. Lozar, *Frame blending with motion vectors* — https://www.klemenlozar.com/frame-blending-with-motion-vectors/
36. SideFX, *Labs Vertex Animation Textures* — https://www.sidefx.com/docs/houdini/nodes/out/labs--vertex_animation_textures-3.0.html
37. AMD GPUOpen, *FSR 3 Unreal Engine plugin guide* — https://gpuopen.com/learn/ue-fsr3/
38. J. Andersson, N. Tatarchuk, *Frostbite Rendering Architecture and Real-time Procedural Shading & Texturing Techniques*, GDC 2007 — https://www.slideshare.net/repii/frostbite-rendering-architecture-and-realtime-procedural-shading-texturing-techniques-presentation
39. U. Haar, S. Aaltonen, *GPU-Driven Rendering Pipelines*, SIGGRAPH 2015 Advances course — https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf
40. NVIDIA, *Vulkan Driver Support* (beta driver notes) — https://developer.nvidia.com/vulkan-driver
41. NVIDIA, *GeForce RTX 3060 family* — https://www.nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti/
42. NVIDIA, *GeForce RTX 3070 family* — https://www.nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3070-3070ti/
43. N. Tatarchuk, C. Tchou, *TFX: Destiny Shader System*, GDC 2017 — https://advances.realtimerendering.com/destiny/gdc_2017/
44. Epic, *Instanced Static Mesh Component* — https://dev.epicgames.com/documentation/en-us/unreal-engine/instanced-static-mesh-component-in-unreal-engine
45. Wikipedia, *GeForce RTX 30 series* (desktop specification table) — https://en.wikipedia.org/wiki/GeForce_RTX_30_series
46. NVIDIA, *CUDA Programming Guide*, compute capabilities appendix — https://docs.nvidia.com/cuda/cuda-programming-guide/05-appendices/compute-capabilities.html

**In-tree, read at `6394bc5e`:**
- `crates/boyko_render/src/{material.rs, material_table.rs, mesh_draw.rs, instance_model.rs, particle_clock.rs, taa_config.rs, upload.rs}`
- `crates/boyko_ecs/src/ecs/core/{asset/assets.rs, time/time.rs}`
- `crates/boyko_rhi/src/{encoder.rs, descriptor.rs}`
- `crates/boyko_rhi_vulkan/src/{device.rs, memory.rs, compute.rs, present/mod.rs, present/passes/{vb.rs, particles.rs, gbuffer.rs, forward.rs}, rhi_impl/device.rs}`
- `crates/boyko_rhi_vulkan/shaders/{vb_classify_common.hlsli, vb_resolve.comp.hlsl, vb_shade.comp.hlsl, deferred_pbr.hlsl, gbuffer_mrt.{vs,fs}.hlsl, forward_opaque.{vs,fs}.hlsl, vb_raster.fs.hlsl}`
- `crates/boyko_app/src/{runner.rs, gpu_scene/mod.rs}`
- `crates/boyko_shaderdsl/src/{emit/mod.rs, emit/vb.rs, interp.rs, bin/emit_particles.rs}`
- `crates/boyko_scene/src/camera.rs`
- `crates/boyko_sdf_math/src/lib.rs`
- `crates/boyko_ui/src/animation.rs`
- `docs/{PBR-MATERIALS-PLAN.md, VB-P2-CLASSIFICATION-PLAN.md, SHADER-VARIANT-MANIFEST.md, RENDER-PARITY-PLAN.md}`
- `docs/render/TRANSPARENCY-DESIGN-SPACE.md`

**Added in pass 2:**
- `crates/boyko_rhi_vulkan/shaders/{vb_classify_count.comp.hlsl, vb_classify_scatter.comp.hlsl, vb_geom_fetch.hlsli, sdf_probe_update.comp.hlsl, sdf_mesh_shadow.comp.hlsl}` and the committed `.spv` sizes beside them
- `crates/boyko_rhi_vulkan/src/present/{targets.rs, passes/gbuffer.rs}`
- `crates/boyko_ui/src/components.rs` (`EasingId`)
- `crates/boyko_app/tests/asset_streaming_f7_grow_headless.rs`
- `crates/boyko_{shaderdsl,ui,render,sdf_math}/Cargo.toml`
- `docs/{PARTICLES-PLAN.md, OPEN-QUESTIONS.md}`
- the same-day sibling designs `docs/render/{VFX-DESIGN-SPACE.md, TRANSPARENCY-UPDATE-2026-09-25.md, REFLECTIONS-UPDATE-2026-09-25.md}`
