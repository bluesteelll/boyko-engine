// Multi-paradigm render-path plan, rung R8: the VisibilityBuffer mesh raster VERTEX shader.
// A POSITION-ONLY clone of `depth_prepass.vs.hlsl`'s instanced arm (the SAME instance-SSBO @0 +
// `base_instance`/`use_model_matrix` push idiom + reverse-Z `pc.view_proj` rows,
// `boyko_render::view::forward_view_proj_rows`, Decision 4), with TWO differences:
//
//   1. **Instance row shape.** Reads `VbInstanceRow` (64 B: the SAME leading 48-byte 3x4
//      row-major affine as `InstanceModelCol`, plus an appended `mesh_id` lane at offset 48 —
//      `boyko_render::instance_model::VbInstanceRow`) instead of the 48-byte `InstanceModelCol`.
//      This VS does not itself read `mesh_id` (the compute fetch, `vb_geom_fetch.hlsli`, reads
//      it back from the SAME SSBO by `instance_id` — no VS export needed).
//   2. **Flat instance-id export.** The GLOBAL instance index (`pc.base_instance + SV_InstanceID`,
//      or — since rung R2d-4 — that same expression read THROUGH the survivor list) is exported as
//      a `nointerpolation` flat interpolant (Decision 9: `SV_InstanceID` is a VS-only system value
//      with no guaranteed FS-side read, so the id is threaded flat instead of recomputed in the
//      fragment stage from a bare push + a nonexistent FS `SV_InstanceID`).
//
// NO jitter IN THIS SHADER at any rung: the TAA-under-VB rungs jitter the HOST-side push
// (`boyko_render::view::forward_view_proj_rows_jittered` perturbs the `view_proj` rows 0/1 when
// TAA is armed; the reverse-Z depth row stays byte-untouched), so this VS is jitter-agnostic —
// it multiplies whatever `pc.view_proj` arrives, jittered or not.
//
// The PUSH CONSTANT layout is byte-identical to `forward_opaque.vs.hlsl`'s / `depth_prepass.vs.hlsl`'s
// 88-byte `{ float4x4 view_proj; float4 cam_eye; uint base_instance; uint use_model_matrix }`
// (`GBUFFER_PUSH_BYTES`) — `cam_eye` is unread by this VS, kept for push-byte-layout parity so the
// SAME host push-encoding machinery is reused verbatim.
//
// ============================================================================================
// VG rung R2d-4 — the per-INSTANCE VISIBLE-LIST INDIRECTION
// ============================================================================================
//
// The instanced arm may read its instance index THROUGH the per-instance survivor list the batch
// cull writes (`vb_batch_cull.comp.hlsl`, rung R2d-3: `VbVisibleInstance[base + k] = <the global
// index of this batch's k-th survivor>`) instead of computing it. The host arms that PER DRAW.
//
// ⚠️ HISTORICAL, and no longer the state of this file. Rung R2d-4 shipped this read INERT: the
// list rung R2d-3 wrote was the IDENTITY (its `keep` was hardwired `true`), so
// `visible_instances[base + id] == base + id` and the indirected expression was literally the
// pre-R2d one. **Rung R2d-6 armed the cull, and every clause of that is now false.**
//
// The armed reality: the list is a COMPACTION. `visible_instances[base + id]` is the global index
// of this batch's id-th SURVIVOR, which is `>= base + id` and generally not equal to it.
// `SV_InstanceID` ranges over `[0, instanceCount)` where `instanceCount` is the survivor count `k`
// the cull itself stored into the record — so this read is confined to `[base, base + k)`, exactly
// the slots the cull wrote this frame, and the slots `[base + k, base + count)` it did not write
// are unreachable from here. The identity survives in exactly one case: a batch nothing rejects.
//
// Kept rather than deleted because the inert form is what every VB golden pin was blessed against
// through rungs R2d-2..R2d-5, and a reader bisecting those pins needs to know why they did not move.
//
// ## The `use_model_matrix` BITFIELD contract
//
// The push word at offset 84 is a BITFIELD, not an enum:
//
//   * bit 0 (value 1) — the ARM selector, semantics UNCHANGED from every rung before this one:
//     `0` selects the legacy arm, NON-ZERO selects the instanced arm. The 88-byte push struct is
//     SHARED, so that has to hold for every consumer, and it was checked rather than assumed: the
//     word is TESTED in exactly six shaders — `vb_raster.vs`, `depth_prepass.vs`,
//     `forward_opaque.vs`, `gbuffer_mrt.vs`, `csm_depth.vs`, `punctual_depth.vs` — and all six
//     spell it `if (pc.use_model_matrix == 0u)`, once each. NONE tests `== 1u`, so a set high bit
//     cannot flip any of them into the wrong arm. (`punctual_depth.fs` declares the field and
//     never references it.) Of those six, only this shader is ever pushed a word with bit 1 set,
//     and the reason SPLITS — stating only half of it would leave the other three unexplained:
//       - `csm_depth.vs` and `punctual_depth.vs` push their OWN 88-byte templates
//         (`CsmActivation::push`, `AtlasActivation::push`), of which every recorder rewrites only
//         `base_instance` at [80,84). They can never observe byte 84 from `scene.mvp` at all.
//       - `depth_prepass.vs`, `forward_opaque.vs` and `gbuffer_mrt.vs` ARE pushed `scene.mvp`
//         verbatim, so for them the guarantee is not structural separation but that the bit is
//         never in `scene.mvp` to begin with: the recorder ORs it into a FUNCTION-LOCAL copy of
//         the word, and `scene.mvp` — reached through a shared `&GBufferScene` — is never mutated.
//         Their own per-batch rewrites touch only [80,84), leaving byte 84 as the host built it.
//     So the safe value 3 is confined to this pipeline's push range by construction. The value
//     that would be dangerous is 2 (bit 1 WITHOUT bit 0), which would flip this shader out of the
//     legacy arm; the recorder makes it unreachable by requiring bit 0 in the predicate that sets
//     bit 1, rather than by asserting after the fact.
//   * bit 1 (value 2) — the VISIBLE-INDIRECTION selector, new this rung. Set by the recorder for
//     a draw whose batch owns a region of `visible_instances` that the cull WROTE this frame;
//     clear otherwise, and clear then means "compute the index", which is the pre-R2d expression
//     evaluated literally.
//
// The reachable values are therefore 0 (legacy), 1 (instanced, direct) and 3 (instanced,
// indirected). 2 would mean indirection without the instanced arm, which is meaningless; the
// recorder `debug_assert!`s that it never assembles one.
//
// ## The indirection is on the LOAD ADDRESS ONLY
//
// It replaces the index used to LOAD the instance row. The row shape, the affine, the matrix
// multiply and the exported id are all unchanged — no branch downstream of `global` is aware of
// which arm produced it.
//
// DXC is free to lower the `? :` below to a BRANCH or to an eager load plus an `OpSelect`, and
// either is safe here: `visible_instances` holds `INSTANCE_CAPACITY` uints, the SAME element count
// as the `instances` ring (`gpu_scene`'s two allocations, `INSTANCE_CAPACITY * 4` and
// `INSTANCE_CAPACITY * 64`), so `pc.base_instance + SV_InstanceID` — an address the instanced arm
// already dereferences unconditionally against `instances` — is in range for BOTH buffers under
// exactly the same condition. An eager load on the not-taken arm therefore reads an indeterminate
// VALUE at an IN-BOUNDS address, which the select discards.
//
// ## ⚠️ INVARIANT R2d-EXPORT-IS-GLOBAL — the exported id is an ORIGINAL GLOBAL INDEX
//
// `output.instance_id` MUST stay an index into the instance ring, never a compacted slot number.
// It is the key that addresses `gVbInstances` in `vb_geom_fetch.hlsli`'s per-pixel geometry
// re-fetch, `instance_materials[...]` in every unpack consumer, and the host readback's
// `(instance_id << 32) | prim` census key. What this rung reads is INDIRECTION, not renumbering:
// `visible_instances[base + k]` STORES a global index, so exporting `global` — the value loaded
// out of the list, not the slot it was loaded from — keeps every one of those consumers keyed on
// exactly the number it was keyed on before. A later rung that redefines the list to hold slot
// numbers breaks all of them at once, silently, behind an image that still looks plausible.
//
// ## ⚠️ `SV_InstanceID`'s LOWERING became load-bearing at this rung
//
// Before this rung `SV_InstanceID` only chose which instance ROW to read, and the host writes
// `first_instance = 0` into every indirect record (`record_vb`'s own ⚠️ on that field, guarded by
// a `debug_assert!` there) because `drawIndirectFirstInstance` is not enabled on this device. An
// id shifted by a nonzero `firstInstance` would then have been a WRONG TRANSFORM — bad pixels,
// bounded memory.
//
// It now also indexes `visible_instances`, whose written region is exactly
// `[base_instance, base_instance + instance_count)`. DXC lowers `SV_InstanceID` to a SPIR-V
// builtin, and WHICH builtin decides whether the value is relative to the draw's `firstInstance`
// or absolute (the `BaseInstance`-subtracting form is only emitted under
// `-fvk-support-nonzero-base-instance`, which the frozen recipe below does not pass). If a later
// rung enables `drawIndirectFirstInstance` and writes a nonzero `firstInstance`, the same shift
// becomes an OUT-OF-RANGE read of this SSBO instead. `robustBufferAccess` is OFF on this device,
// so that read is undefined rather than clamped, and neither the goldens nor the validation
// layers can see it. `tests/vb_raster_geo_classify_spv_sync.rs` pins which builtin the COMMITTED
// module decorates, so such a rung goes RED there rather than shipping.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_raster.vs.hlsl -Fo vb_raster.vs.spv

struct PushConstants {
    float4x4 view_proj;        // reverse-Z proj*view, column-major (boyko_render::view::forward_view_proj_rows)
    float4   cam_eye;          // unread by this position-only pass; kept for push-byte-layout parity
    uint     base_instance;    // the SSBO bucket base: index instances[base_instance + SV_InstanceID]
    uint     use_model_matrix; // BITFIELD: bit 0 = instanced arm (0 = legacy); bit 1 = visible-indirection
};
[[vk::push_constant]] PushConstants pc;

// Decision 0's VB-path instance row (64 B) — byte-identical leading bytes to `InstanceModelCol`
// (offset 0..48), plus the appended `mesh_id` lane (offset 48), a per-instance `flags` word
// (offset 52) and an 8-byte pad (offset 56..64, std430 stability). This VS reads ONLY
// `r0`/`r1`/`r2` — `mesh_id`/`_pad` are declared for byte layout parity but never referenced
// here (the compute fetch reads them back by `instance_id`).
//
// VG R3 piece 2 step P2-2 made the host's word @52 a FLAGS word (bit 0 = "this instance's
// entity carries `OcclusionCulling`"; bits 1..31 reserved, written zero) where it was
// `_pad[0]`. The `uint3 _pad` spelling below covers offsets 52..64 and is layout-identical
// either way, so it is deliberately NOT renamed here — piece 3 renames it in the same change
// that first reads the bit. Read by nothing on the device as of P2-2.
struct VbInstanceRow {
    float4 r0;
    float4 r1;
    float4 r2;
    uint   mesh_id;
    uint3  _pad;
};
[[vk::binding(0, 0)]] StructuredBuffer<VbInstanceRow> instances;

// VG rung R2d-4: the per-INSTANCE survivor list, at the VB Set-0 binding rung R2d-2 allocated for
// it (@11 — VERTEX-stage STORAGE_BUFFER; @11 rather than @8 because @8/@9 are the froxel pair in
// `vb_layout0_froxel` and @10 is held for VB-SV0, so ONE number is free in both VB Set-0 layouts
// and this single compiled module can name it under either). One `uint` per instance slot,
// written by `vb_batch_cull.comp.hlsl` into each batch's OWNED, disjoint region
// `[base_instance, base_instance + survivors)` — see that shader's INVARIANT R2d-REGION-DISJOINT.
// The stored value is an ORIGINAL GLOBAL instance index (INVARIANT R2d-EXPORT-IS-GLOBAL above).
[[vk::binding(11, 0)]] StructuredBuffer<uint> visible_instances;

// Field DECLARATION order fixes the SPIR-V vertex-input locations DXC auto-assigns — the SAME
// order every other raster VS in this codebase uses (position@0/normal@12/color@24 in the
// `boyko_render::mesh::Vertex` 64-byte stride); normal/color are declared for `VertexAttribute`
// parity but unread by this position-only pass.
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1 (unread)
    float3 normal   : NORMAL;    // SPIR-V location 2 (unread)
};

struct VsOut {
    float4 position : SV_Position;
    nointerpolation uint instance_id : IID; // the GLOBAL instance index (Decision 9 / R2d-EXPORT-IS-GLOBAL)
};

VsOut main(VsIn input, uint instance_id : SV_InstanceID) {
    VsOut output;
    if (pc.use_model_matrix == 0u) {
        // LEGACY arm — a merged (non-instanced) draw. `input.position` IS the world position;
        // no per-instance row exists for this arm (mirrors every other raster VS's legacy arm).
        output.position = mul(pc.view_proj, float4(input.position, 1.0));
        output.instance_id = 0u;
        return output;
    }
    // INSTANCED arm — read the per-instance 3x4 row-major affine and place the vertex in world
    // space (byte-identical construction to `forward_opaque.vs.hlsl`'s instanced arm).
    //
    // Rung R2d-4: `global` is the ONE index this lane uses — for the row LOAD and for the export.
    // With bit 1 clear it is the pre-R2d expression verbatim; with it set it is the same
    // expression used as the ADDRESS of a survivor-list entry whose stored value is a global
    // index. Both arms therefore yield a global index, which is what INVARIANT
    // R2d-EXPORT-IS-GLOBAL requires of everything downstream.
    uint global = ((pc.use_model_matrix & 2u) != 0u)
        ? visible_instances[pc.base_instance + instance_id]
        : (pc.base_instance + instance_id);
    VbInstanceRow model = instances[global];
    float3x3 m3 = float3x3(model.r0.xyz, model.r1.xyz, model.r2.xyz);
    float3 t = float3(model.r0.w, model.r1.w, model.r2.w);
    float3 world = mul(m3, input.position) + t;
    output.position = mul(pc.view_proj, float4(world, 1.0));
    output.instance_id = global;
    return output;
}
