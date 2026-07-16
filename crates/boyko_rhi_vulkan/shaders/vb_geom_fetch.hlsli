// Multi-paradigm render-path plan, rung R8 (Decision 0 / Decision 9 / plan §F): the
// Visibility-Buffer per-pixel geometry re-fetch — `(instance_id, raw_prim_id)` unpacked from
// `vb_id` -> the SAME `Surface`-ready attribute bundle a raster VS/FS pair would have
// interpolated for free, reconstructed here via the bindless per-mesh geometry table
// (Decision 0) + the analytic screen-space barycentric math (`boyko_shaderdsl::vb`, rung R7).
//
// # INCLUDE CONTRACT
//
// Self-contained: declares its OWN Set-2 bindings (`gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta`
// — the VB-only geometry table, a DISTINCT set from the shared bindless TEXTURE table, Decision
// 0's P2-c placement) and needs nothing pre-declared by the caller. `#include` this file AFTER
// `vb_pack.hlsli` (for `VbId`/`VB_ID_SENTINEL`, not referenced by this file's own functions but
// conventionally included first by every consumer) and the `VbInstanceRow` shape below (a
// second, HLSL-side mirror of `boyko_render::instance_model::VbInstanceRow` — every raster/
// compute pass in this pipeline family declares its OWN copy of the shared host structs, the
// SAME "fixed names/shapes, no cross-shader-file struct sharing" discipline this codebase's
// other `.hlsli`s already establish, e.g. `MaterialGpu` re-declared verbatim in
// `forward_opaque.fs.hlsl` AND `sdf_forward_march.comp.hlsl`).
//
// # Set-numbering deviation from the plan's literal "Set 3" text (documented, precedented)
//
// The plan's aspirational numbering assumed Set 1 = the shared bindless TEXTURE table (unused
// by the non-textured v1 VB pipeline) and Set 2 = shadow, landing geometry at Set 3. The ACTUAL
// Forward-family implementation never wires a bindless-texture set at all (`forward_opaque.fs.hlsl`'s
// own doc: "this v1 pipeline has no texture table at all") and renumbered its shadow set from the
// plan's Set 2 to Set 1 for the SAME reason `ForwardTargets::set1`'s doc gives: a zero-binding
// PLACEHOLDER layout (needed to keep an empty Set 1 contiguous) is REJECTED by
// `create_bind_group_layout`'s own `1..=MAX_BIND_GROUP_BINDINGS` invariant. This module makes the
// IDENTICAL renumbering choice for the SAME reason: `vb_resolve`'s pipeline layout is `[Set0
// core, Set1 shadow (REUSED verbatim from `ForwardTargets`'s layout object), Set2 geometry]` — a
// real, contiguous 3-set layout, not a 4-set one with an empty Set 2 placeholder. The DESIGN
// intent Decision 0/P2-c protects — the geometry arrays are a VB-only set, never appended to the
// shared bindless texture set — is preserved exactly; only the literal set INDEX differs from
// the plan's aspirational text, mirroring an ALREADY-ESTABLISHED precedent in this exact codebase.
//
// # Vertex layout pin (Decision 0's geometry-table sync-pin)
//
// `Vertex` (host: `boyko_render::mesh::Vertex`, 64 B): `position` @0 (float3), `normal` @12
// (float3), `color` @24 (float4), `uv` @40 (float2), `tangent` @48 (float4). This file reads
// `position`/`normal`/`color`/`uv` unconditionally; TV0 rung (`RENDER-PARITY-PLAN.md` §2.3)
// ALSO reads `tangent` under `#ifdef TEXTURED` (the world-tangent + handedness the TBN normal
// map needs) — the base (non-`TEXTURED`) compile leaves it unread, byte-frozen.

struct VbInstanceRow {
    float4 r0;
    float4 r1;
    float4 r2;
    uint   mesh_id;
    uint3  _pad;
};
[[vk::binding(0, 0)]] StructuredBuffer<VbInstanceRow> gVbInstances;

// One `gMeshMeta[]` row (Decision 0 / `boyko_render::mesh_geometry_table::MeshGeometryMeta`,
// 16 B): `index_width` (bytes: 2 or 4), `vertex_count`, `index_count`, `_pad`.
struct MeshGeometryMeta {
    uint index_width;
    uint vertex_count;
    uint index_count;
    uint _pad;
};

// Decision 0 / P2-c: the VB-only Set-2 geometry table — two bindless `STORAGE_BUFFER` runtime
// arrays (one slot per registered mesh, index = `mesh_id`) plus the plain (non-bindless)
// `gMeshMeta[]` SSBO. Binding numbers mirror `boyko_rhi_vulkan::geometry_bindless`'s
// `GEOMETRY_VERTS_BINDING`/`GEOMETRY_INDICES_BINDING`/`GEOMETRY_META_BINDING` verbatim (0/1/2).
[[vk::binding(0, 2)]] ByteAddressBuffer gMeshVerts[];
[[vk::binding(1, 2)]] ByteAddressBuffer gMeshIndices[];
[[vk::binding(2, 2)]] StructuredBuffer<MeshGeometryMeta> gMeshMeta;

// Decision 9's triangle-count normalizer — `tri_count = index_count / 3` (every mesh draws a
// plain non-restart triangle list). Byte-identical formula to the host mirror
// `boyko_render::mesh_geometry_table::tri_count`.
uint vb_tri_count(uint index_count) {
    return index_count / 3u;
}

// Loads ONE triangle-list index at `idx` from mesh `mesh_id`'s index buffer, honoring the
// per-mesh `index_width` (2 = `Uint16`, 4 = `Uint32` — Decision 0's `gMeshMeta[].index_width`
// encoding). `ByteAddressBuffer` has no native 16-bit load, so the `Uint16` arm loads the
// containing 4-byte-aligned word and extracts the low/high half (even/odd `idx`).
uint vb_load_index(uint mesh_id, uint idx, uint index_width) {
    ByteAddressBuffer ib = gMeshIndices[NonUniformResourceIndex(mesh_id)];
    if (index_width == 2u) {
        uint word_offset = (idx / 2u) * 4u;
        uint word = ib.Load(word_offset);
        return ((idx & 1u) == 0u) ? (word & 0xFFFFu) : (word >> 16u);
    }
    return ib.Load(idx * 4u);
}

// One re-fetched mesh vertex (the fields this pipeline actually reads — `tangent` is skipped
// under the base (non-`TEXTURED`) compile; TV0's `#ifdef TEXTURED` adds it back for tangent-
// space normal mapping, mirroring `gbuffer_mrt.vs.hlsl`'s own `TEXTURED`-only `tangent` read).
struct VbVertex {
    float3 position;
    float3 normal;
    float4 color;
    float2 uv;
#ifdef TEXTURED
    float4 tangent;
#endif
};

// Loads mesh `mesh_id`'s vertex `vidx` at the 64-byte `Vertex` stride (this file's own doc pins
// the offsets: position@0/normal@12/color@24/uv@40/tangent@48).
VbVertex vb_load_vertex(uint mesh_id, uint vidx) {
    ByteAddressBuffer vb = gMeshVerts[NonUniformResourceIndex(mesh_id)];
    uint base = vidx * 64u;
    VbVertex v;
    v.position = asfloat(vb.Load3(base + 0u));
    v.normal   = asfloat(vb.Load3(base + 12u));
    v.color    = asfloat(vb.Load4(base + 24u));
    v.uv       = asfloat(vb.Load2(base + 40u));
#ifdef TEXTURED
    v.tangent  = asfloat(vb.Load4(base + 48u));
#endif
    return v;
}

// === GENERATED vb_barycentric_grad BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_barycentric)
struct VbBaryGrad { float3 dlambda_dx; float3 dlambda_dy; };
VbBaryGrad vb_barycentric_grad(float3 vx, float3 vy) {
    float t0 = vx.z - vx.y;
    float t1 = vy.x - vy.y;
    float t2 = t0 * t1;
    float t3 = vy.z - vy.y;
    float t4 = vx.x - vx.y;
    float t5 = t3 * t4;
    float t6 = t2 - t5;
    float t7 = 1.0 / t6;
    float t8 = vy.y - vy.z;
    float t9 = t8 * t7;
    float t10 = vy.z - vy.x;
    float t11 = t10 * t7;
    float t12 = vy.x - vy.y;
    float t13 = t12 * t7;
    float t14 = vx.z - vx.y;
    float t15 = t14 * t7;
    float t16 = vx.x - vx.z;
    float t17 = t16 * t7;
    float t18 = vx.y - vx.x;
    float t19 = t18 * t7;
    VbBaryGrad g;
    g.dlambda_dx = float3(t9, t11, t13);
    g.dlambda_dy = float3(t15, t17, t19);
    return g;
}
// === GENERATED vb_barycentric_grad END ===

// === GENERATED vb_barycentric_eval BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_barycentric)
float3 vb_barycentric_eval(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, float px, float py) {
    float t0 = px - x0;
    float t1 = py - y0;
    float t2 = dlambda_dx.x * t0;
    float t3 = 1.0 + t2;
    float t4 = dlambda_dy.x * t1;
    float t5 = t3 + t4;
    float t6 = dlambda_dx.y * t0;
    float t7 = dlambda_dy.y * t1;
    float t8 = t6 + t7;
    float t9 = dlambda_dx.z * t0;
    float t10 = dlambda_dy.z * t1;
    float t11 = t9 + t10;
    return float3(t5, t8, t11);
}
// === GENERATED vb_barycentric_eval END ===

// === GENERATED vb_interp BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_interp)
float3 vb_interp(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, float px, float py, float3 a, float3 w) {
    float t0 = px - x0;
    float t1 = py - y0;
    float t2 = 1.0 / w.x;
    float t3 = 1.0 / w.y;
    float t4 = 1.0 / w.z;
    float t5 = a.x * t2;
    float t6 = a.y * t3;
    float t7 = a.z * t4;
    float t8 = dlambda_dx.x * t5;
    float t9 = dlambda_dx.y * t6;
    float t10 = t8 + t9;
    float t11 = dlambda_dx.z * t7;
    float t12 = t10 + t11;
    float t13 = dlambda_dy.x * t5;
    float t14 = dlambda_dy.y * t6;
    float t15 = t13 + t14;
    float t16 = dlambda_dy.z * t7;
    float t17 = t15 + t16;
    float t18 = dlambda_dx.x * t2;
    float t19 = dlambda_dx.y * t3;
    float t20 = t18 + t19;
    float t21 = dlambda_dx.z * t4;
    float t22 = t20 + t21;
    float t23 = dlambda_dy.x * t2;
    float t24 = dlambda_dy.y * t3;
    float t25 = t23 + t24;
    float t26 = dlambda_dy.z * t4;
    float t27 = t25 + t26;
    float t28 = t0 * t12;
    float t29 = t5 + t28;
    float t30 = t1 * t17;
    float t31 = t29 + t30;
    float t32 = t0 * t22;
    float t33 = t2 + t32;
    float t34 = t1 * t27;
    float t35 = t33 + t34;
    float t36 = t31 / t35;
    float t37 = t35 * t35;
    float t38 = t12 * t35;
    float t39 = t31 * t22;
    float t40 = t38 - t39;
    float t41 = t40 / t37;
    float t42 = t17 * t35;
    float t43 = t31 * t27;
    float t44 = t42 - t43;
    float t45 = t44 / t37;
    return float3(t36, t41, t45);
}
// === GENERATED vb_interp END ===

// === GENERATED vb_uv_grad BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_uv_grad)
float4 vb_uv_grad(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, float px, float py, float3 u, float3 v, float3 w) {
    float t0 = px - x0;
    float t1 = py - y0;
    float t2 = 1.0 / w.x;
    float t3 = 1.0 / w.y;
    float t4 = 1.0 / w.z;
    float t5 = u.x * t2;
    float t6 = u.y * t3;
    float t7 = u.z * t4;
    float t8 = dlambda_dx.x * t5;
    float t9 = dlambda_dx.y * t6;
    float t10 = t8 + t9;
    float t11 = dlambda_dx.z * t7;
    float t12 = t10 + t11;
    float t13 = dlambda_dy.x * t5;
    float t14 = dlambda_dy.y * t6;
    float t15 = t13 + t14;
    float t16 = dlambda_dy.z * t7;
    float t17 = t15 + t16;
    float t18 = dlambda_dx.x * t2;
    float t19 = dlambda_dx.y * t3;
    float t20 = t18 + t19;
    float t21 = dlambda_dx.z * t4;
    float t22 = t20 + t21;
    float t23 = dlambda_dy.x * t2;
    float t24 = dlambda_dy.y * t3;
    float t25 = t23 + t24;
    float t26 = dlambda_dy.z * t4;
    float t27 = t25 + t26;
    float t28 = t0 * t12;
    float t29 = t5 + t28;
    float t30 = t1 * t17;
    float t31 = t29 + t30;
    float t32 = t0 * t22;
    float t33 = t2 + t32;
    float t34 = t1 * t27;
    float t35 = t33 + t34;
    float t36 = t31 / t35;
    float t37 = t35 * t35;
    float t38 = t12 * t35;
    float t39 = t31 * t22;
    float t40 = t38 - t39;
    float t41 = t40 / t37;
    float t42 = t17 * t35;
    float t43 = t31 * t27;
    float t44 = t42 - t43;
    float t45 = t44 / t37;
    float t46 = px - x0;
    float t47 = py - y0;
    float t48 = 1.0 / w.x;
    float t49 = 1.0 / w.y;
    float t50 = 1.0 / w.z;
    float t51 = v.x * t48;
    float t52 = v.y * t49;
    float t53 = v.z * t50;
    float t54 = dlambda_dx.x * t51;
    float t55 = dlambda_dx.y * t52;
    float t56 = t54 + t55;
    float t57 = dlambda_dx.z * t53;
    float t58 = t56 + t57;
    float t59 = dlambda_dy.x * t51;
    float t60 = dlambda_dy.y * t52;
    float t61 = t59 + t60;
    float t62 = dlambda_dy.z * t53;
    float t63 = t61 + t62;
    float t64 = dlambda_dx.x * t48;
    float t65 = dlambda_dx.y * t49;
    float t66 = t64 + t65;
    float t67 = dlambda_dx.z * t50;
    float t68 = t66 + t67;
    float t69 = dlambda_dy.x * t48;
    float t70 = dlambda_dy.y * t49;
    float t71 = t69 + t70;
    float t72 = dlambda_dy.z * t50;
    float t73 = t71 + t72;
    float t74 = t46 * t58;
    float t75 = t51 + t74;
    float t76 = t47 * t63;
    float t77 = t75 + t76;
    float t78 = t46 * t68;
    float t79 = t48 + t78;
    float t80 = t47 * t73;
    float t81 = t79 + t80;
    float t82 = t77 / t81;
    float t83 = t81 * t81;
    float t84 = t58 * t81;
    float t85 = t77 * t68;
    float t86 = t84 - t85;
    float t87 = t86 / t83;
    float t88 = t63 * t81;
    float t89 = t77 * t73;
    float t90 = t88 - t89;
    float t91 = t90 / t83;
    return float4(t41, t45, t87, t91);
}
// === GENERATED vb_uv_grad END ===

// === GENERATED vb_near_clip BEGIN === (boyko_shaderdsl::emit::emit_hlsl_vb_near_clip)
struct VbClippedTri { float4 v0; float4 v1; float4 v2; };
VbClippedTri vb_near_clip(float4 v0, float4 v1, float4 v2) {
    float t0 = 0.0001 - v0.w;
    float t1 = v1.w - v0.w;
    float t2 = abs(t1);
    float t3 = max(t2, 1.0e-6);
    float t4 = -t3;
    float t5 = (t1 < 0.0) ? t4 : t3;
    float t6 = t0 / t5;
    float t7 = clamp(t6, 0.0, 1.0);
    float t8 = lerp(v0.x, v1.x, t7);
    float t9 = lerp(v0.y, v1.y, t7);
    float t10 = lerp(v0.z, v1.z, t7);
    float t11 = lerp(v0.w, v1.w, t7);
    float t12 = 0.0001 - v0.w;
    float t13 = v2.w - v0.w;
    float t14 = abs(t13);
    float t15 = max(t14, 1.0e-6);
    float t16 = -t15;
    float t17 = (t13 < 0.0) ? t16 : t15;
    float t18 = t12 / t17;
    float t19 = clamp(t18, 0.0, 1.0);
    float t20 = lerp(v0.x, v2.x, t19);
    float t21 = lerp(v0.y, v2.y, t19);
    float t22 = lerp(v0.z, v2.z, t19);
    float t23 = lerp(v0.w, v2.w, t19);
    float t24 = t8 + t20;
    float t25 = t24 * 0.5;
    float t26 = t9 + t21;
    float t27 = t26 * 0.5;
    float t28 = t10 + t22;
    float t29 = t28 * 0.5;
    float t30 = t11 + t23;
    float t31 = t30 * 0.5;
    float t32 = max(v0.w, 0.0001);
    float t33 = (v2.w > 0.0001) ? t25 : t8;
    float t34 = (v2.w > 0.0001) ? t27 : t9;
    float t35 = (v2.w > 0.0001) ? t29 : t10;
    float t36 = (v2.w > 0.0001) ? t31 : t11;
    float t37 = (v2.w > 0.0001) ? t20 : v0.x;
    float t38 = (v2.w > 0.0001) ? t21 : v0.y;
    float t39 = (v2.w > 0.0001) ? t22 : v0.z;
    float t40 = (v2.w > 0.0001) ? t23 : t32;
    float t41 = (v1.w > 0.0001) ? t33 : t37;
    float t42 = (v1.w > 0.0001) ? t34 : t38;
    float t43 = (v1.w > 0.0001) ? t35 : t39;
    float t44 = (v1.w > 0.0001) ? t36 : t40;
    float t45 = (v0.w <= 0.0001) ? t41 : v0.x;
    float t46 = (v0.w <= 0.0001) ? t42 : v0.y;
    float t47 = (v0.w <= 0.0001) ? t43 : v0.z;
    float t48 = (v0.w <= 0.0001) ? t44 : v0.w;
    float t49 = 0.0001 - v1.w;
    float t50 = v2.w - v1.w;
    float t51 = abs(t50);
    float t52 = max(t51, 1.0e-6);
    float t53 = -t52;
    float t54 = (t50 < 0.0) ? t53 : t52;
    float t55 = t49 / t54;
    float t56 = clamp(t55, 0.0, 1.0);
    float t57 = lerp(v1.x, v2.x, t56);
    float t58 = lerp(v1.y, v2.y, t56);
    float t59 = lerp(v1.z, v2.z, t56);
    float t60 = lerp(v1.w, v2.w, t56);
    float t61 = 0.0001 - v1.w;
    float t62 = v0.w - v1.w;
    float t63 = abs(t62);
    float t64 = max(t63, 1.0e-6);
    float t65 = -t64;
    float t66 = (t62 < 0.0) ? t65 : t64;
    float t67 = t61 / t66;
    float t68 = clamp(t67, 0.0, 1.0);
    float t69 = lerp(v1.x, v0.x, t68);
    float t70 = lerp(v1.y, v0.y, t68);
    float t71 = lerp(v1.z, v0.z, t68);
    float t72 = lerp(v1.w, v0.w, t68);
    float t73 = t57 + t69;
    float t74 = t73 * 0.5;
    float t75 = t58 + t70;
    float t76 = t75 * 0.5;
    float t77 = t59 + t71;
    float t78 = t77 * 0.5;
    float t79 = t60 + t72;
    float t80 = t79 * 0.5;
    float t81 = max(v1.w, 0.0001);
    float t82 = (v0.w > 0.0001) ? t74 : t57;
    float t83 = (v0.w > 0.0001) ? t76 : t58;
    float t84 = (v0.w > 0.0001) ? t78 : t59;
    float t85 = (v0.w > 0.0001) ? t80 : t60;
    float t86 = (v0.w > 0.0001) ? t69 : v1.x;
    float t87 = (v0.w > 0.0001) ? t70 : v1.y;
    float t88 = (v0.w > 0.0001) ? t71 : v1.z;
    float t89 = (v0.w > 0.0001) ? t72 : t81;
    float t90 = (v2.w > 0.0001) ? t82 : t86;
    float t91 = (v2.w > 0.0001) ? t83 : t87;
    float t92 = (v2.w > 0.0001) ? t84 : t88;
    float t93 = (v2.w > 0.0001) ? t85 : t89;
    float t94 = (v1.w <= 0.0001) ? t90 : v1.x;
    float t95 = (v1.w <= 0.0001) ? t91 : v1.y;
    float t96 = (v1.w <= 0.0001) ? t92 : v1.z;
    float t97 = (v1.w <= 0.0001) ? t93 : v1.w;
    float t98 = 0.0001 - v2.w;
    float t99 = v0.w - v2.w;
    float t100 = abs(t99);
    float t101 = max(t100, 1.0e-6);
    float t102 = -t101;
    float t103 = (t99 < 0.0) ? t102 : t101;
    float t104 = t98 / t103;
    float t105 = clamp(t104, 0.0, 1.0);
    float t106 = lerp(v2.x, v0.x, t105);
    float t107 = lerp(v2.y, v0.y, t105);
    float t108 = lerp(v2.z, v0.z, t105);
    float t109 = lerp(v2.w, v0.w, t105);
    float t110 = 0.0001 - v2.w;
    float t111 = v1.w - v2.w;
    float t112 = abs(t111);
    float t113 = max(t112, 1.0e-6);
    float t114 = -t113;
    float t115 = (t111 < 0.0) ? t114 : t113;
    float t116 = t110 / t115;
    float t117 = clamp(t116, 0.0, 1.0);
    float t118 = lerp(v2.x, v1.x, t117);
    float t119 = lerp(v2.y, v1.y, t117);
    float t120 = lerp(v2.z, v1.z, t117);
    float t121 = lerp(v2.w, v1.w, t117);
    float t122 = t106 + t118;
    float t123 = t122 * 0.5;
    float t124 = t107 + t119;
    float t125 = t124 * 0.5;
    float t126 = t108 + t120;
    float t127 = t126 * 0.5;
    float t128 = t109 + t121;
    float t129 = t128 * 0.5;
    float t130 = max(v2.w, 0.0001);
    float t131 = (v1.w > 0.0001) ? t123 : t106;
    float t132 = (v1.w > 0.0001) ? t125 : t107;
    float t133 = (v1.w > 0.0001) ? t127 : t108;
    float t134 = (v1.w > 0.0001) ? t129 : t109;
    float t135 = (v1.w > 0.0001) ? t118 : v2.x;
    float t136 = (v1.w > 0.0001) ? t119 : v2.y;
    float t137 = (v1.w > 0.0001) ? t120 : v2.z;
    float t138 = (v1.w > 0.0001) ? t121 : t130;
    float t139 = (v0.w > 0.0001) ? t131 : t135;
    float t140 = (v0.w > 0.0001) ? t132 : t136;
    float t141 = (v0.w > 0.0001) ? t133 : t137;
    float t142 = (v0.w > 0.0001) ? t134 : t138;
    float t143 = (v2.w <= 0.0001) ? t139 : v2.x;
    float t144 = (v2.w <= 0.0001) ? t140 : v2.y;
    float t145 = (v2.w <= 0.0001) ? t141 : v2.z;
    float t146 = (v2.w <= 0.0001) ? t142 : v2.w;
    VbClippedTri c;
    c.v0 = float4(t45, t46, t47, t48);
    c.v1 = float4(t94, t95, t96, t97);
    c.v2 = float4(t143, t144, t145, t146);
    return c;
}
// === GENERATED vb_near_clip END ===

// The per-pixel result `vb_geom_fetch` reconstructs — the SAME per-fragment attribute set a
// raster VS/FS pair would have interpolated for free (Decision 0's re-fetch trade-off).
struct VbGeomFetchResult {
    float3 world_pos;
    float3 world_normal;
    float4 vertex_color;
    float2 uv;
#ifdef TEXTURED
    // TV0 (`RENDER-PARITY-PLAN.md` §2.3): the perspective-correct interpolated world tangent
    // (Gram-Schmidt-corrected against the shading normal at the sample site, mirroring
    // `gbuffer_mrt.fs.hlsl`'s TBN build), the FLAT (nearest-vertex, never interpolated)
    // handedness sign (matches `gbuffer_mrt.vs.hlsl`'s `nointerpolation tex_w`), and the UV
    // screen-space derivative pair `vb_uv_grad` emits — `(du/dx, du/dy, dv/dx, dv/dy)`, ready to
    // split into `SampleGrad`'s `ddx`/`ddy` arguments.
    float3 world_tangent;
    float  tex_w;
    float4 uv_grad;
#endif
};

// The Decision-0/Decision-9 per-pixel geometry re-fetch: `(instance_id, raw_prim_id)` ->
// `mesh_id` (via the instance row) -> `tri_count`/`index_width` (via `gMeshMeta`) -> 3 indices
// -> 3 vertices -> transform to CLIP via `view_proj` + the instance's own world affine ->
// `vb_near_clip` -> project to screen (pixel space, `SV_Position.xy` convention: origin
// top-left, x right, y down — this engine's plain positive-height `VkViewport`, no negative-
// height flip trick, so `screen = (ndc*0.5+0.5)*extent` needs no additional sign flip) ->
// `vb_barycentric_grad` -> `vb_interp` per attribute channel (world position, world normal,
// vertex color, uv — ALL perspective-correct via the SAME McLaren/Hill-corrected form, Decision
// 9's `boyko_shaderdsl::vb` doc). `extent` is the dispatch's `(width, height)` in pixels —
// threaded as a plain parameter (not a global) so this fn stays a pure, testable leaf.
//
// The caller (`vb_resolve.comp.hlsl`) MUST have already excluded `instance_id ==
// VB_ID_SENTINEL` (the SDF-owned / unwritten-pixel case) before calling this fn — this fn
// itself has no SDF/sentinel awareness.
//
// `tri_count` is clamped to at least 1 (a defensive, branch-cheap guard against a genuinely
// malformed/degenerate mesh's `%0`, which is GPU-undefined) — by construction (Decision 9) a
// real rasterized pixel's `mesh_id` always resolves to a non-degenerate mesh with `tri_count >
// 0` (a 0-index mesh draws no primitives, so no pixel ever carries its `mesh_id`); the clamp
// makes that invariant explicit rather than relying on it silently.
VbGeomFetchResult vb_geom_fetch(uint instance_id, uint raw_prim_id, float2 pixel_xy, float4x4 view_proj, float2 extent) {
    VbInstanceRow inst = gVbInstances[instance_id];
    uint mesh_id = inst.mesh_id;
    MeshGeometryMeta meta = gMeshMeta[mesh_id];
    uint tri_count = max(vb_tri_count(meta.index_count), 1u);
    uint local_tri = raw_prim_id % tri_count;

    uint i0 = vb_load_index(mesh_id, local_tri * 3u + 0u, meta.index_width);
    uint i1 = vb_load_index(mesh_id, local_tri * 3u + 1u, meta.index_width);
    uint i2 = vb_load_index(mesh_id, local_tri * 3u + 2u, meta.index_width);

    VbVertex v0 = vb_load_vertex(mesh_id, i0);
    VbVertex v1 = vb_load_vertex(mesh_id, i1);
    VbVertex v2 = vb_load_vertex(mesh_id, i2);

    // The instance's own model affine (byte-identical row-major 3x4 to `InstanceModelCol`'s
    // convention every other raster VS in this codebase uses).
    float3x3 m3 = float3x3(inst.r0.xyz, inst.r1.xyz, inst.r2.xyz);
    float3 t = float3(inst.r0.w, inst.r1.w, inst.r2.w);

    float3 world_p0 = mul(m3, v0.position) + t;
    float3 world_p1 = mul(m3, v1.position) + t;
    float3 world_p2 = mul(m3, v2.position) + t;
    // Documented v1 simplification: the world normal uses the plain linear 3x3 (no
    // inverse-transpose non-uniform-scale correction, unlike `forward_opaque.vs.hlsl`'s M4
    // guard) — correct for uniform-scale instances (the golden scene's spheres), a known
    // limitation for non-uniform scale, deferred to a later rung.
    float3 world_n0 = mul(m3, v0.normal);
    float3 world_n1 = mul(m3, v1.normal);
    float3 world_n2 = mul(m3, v2.normal);
#ifdef TEXTURED
    // TV0: the world tangent transforms with the PLAIN model 3x3 (a surface vector, not a
    // normal — the SAME `m3` reuse `gbuffer_mrt.vs.hlsl`'s TEXTURED arm documents, never the
    // inverse-transpose normal matrix).
    float3 world_t0 = mul(m3, v0.tangent.xyz);
    float3 world_t1 = mul(m3, v1.tangent.xyz);
    float3 world_t2 = mul(m3, v2.tangent.xyz);
#endif

    float4 clip0 = mul(view_proj, float4(world_p0, 1.0));
    float4 clip1 = mul(view_proj, float4(world_p1, 1.0));
    float4 clip2 = mul(view_proj, float4(world_p2, 1.0));

    VbClippedTri clipped = vb_near_clip(clip0, clip1, clip2);

    float3 ndc0 = clipped.v0.xyz / clipped.v0.w;
    float3 ndc1 = clipped.v1.xyz / clipped.v1.w;
    float3 ndc2 = clipped.v2.xyz / clipped.v2.w;

    // NDC -> pixel space: this engine's raster pipelines use a plain positive-height
    // `VkViewport` (no negative-height flip trick — `record_forward.rs`'s `forward_viewport`),
    // so Vulkan's own Y-down clip-space convention needs no additional sign flip here; this
    // reproduces the SAME mapping the rasterizer applies to produce `SV_Position.xy`.
    float sx0 = (ndc0.x * 0.5 + 0.5) * extent.x;
    float sy0 = (ndc0.y * 0.5 + 0.5) * extent.y;
    float sx1 = (ndc1.x * 0.5 + 0.5) * extent.x;
    float sy1 = (ndc1.y * 0.5 + 0.5) * extent.y;
    float sx2 = (ndc2.x * 0.5 + 0.5) * extent.x;
    float sy2 = (ndc2.y * 0.5 + 0.5) * extent.y;

    VbBaryGrad grad = vb_barycentric_grad(float3(sx0, sx1, sx2), float3(sy0, sy1, sy2));
    float3 w3 = float3(clipped.v0.w, clipped.v1.w, clipped.v2.w);

    VbGeomFetchResult result;
    result.world_pos.x = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_p0.x, world_p1.x, world_p2.x), w3).x;
    result.world_pos.y = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_p0.y, world_p1.y, world_p2.y), w3).x;
    result.world_pos.z = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_p0.z, world_p1.z, world_p2.z), w3).x;
    result.world_normal.x = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_n0.x, world_n1.x, world_n2.x), w3).x;
    result.world_normal.y = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_n0.y, world_n1.y, world_n2.y), w3).x;
    result.world_normal.z = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_n0.z, world_n1.z, world_n2.z), w3).x;
    result.vertex_color.x = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(v0.color.x, v1.color.x, v2.color.x), w3).x;
    result.vertex_color.y = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(v0.color.y, v1.color.y, v2.color.y), w3).x;
    result.vertex_color.z = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(v0.color.z, v1.color.z, v2.color.z), w3).x;
    result.vertex_color.w = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(v0.color.w, v1.color.w, v2.color.w), w3).x;
    // `uv` is reconstructed here (perspective-correct, via `vb_interp`) regardless of
    // `TEXTURED` — v1's non-textured material path never samples a texture, so it is left
    // unread by the base shading tail (`vb_resolve.comp.hlsl`).
    result.uv.x = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(v0.uv.x, v1.uv.x, v2.uv.x), w3).x;
    result.uv.y = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(v0.uv.y, v1.uv.y, v2.uv.y), w3).x;
#ifdef TEXTURED
    result.world_tangent.x = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_t0.x, world_t1.x, world_t2.x), w3).x;
    result.world_tangent.y = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_t0.y, world_t1.y, world_t2.y), w3).x;
    result.world_tangent.z = vb_interp(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(world_t0.z, world_t1.z, world_t2.z), w3).x;
    // Flat (nearest-vertex, v0 — the provoking vertex) handedness sign, never interpolated —
    // matches `gbuffer_mrt.vs.hlsl`'s `nointerpolation tex_w` convention.
    result.tex_w = v0.tangent.w;
    result.uv_grad = vb_uv_grad(grad.dlambda_dx, grad.dlambda_dy, sx0, sy0, pixel_xy.x, pixel_xy.y, float3(v0.uv.x, v1.uv.x, v2.uv.x), float3(v0.uv.y, v1.uv.y, v2.uv.y), w3);
#endif
    return result;
}
