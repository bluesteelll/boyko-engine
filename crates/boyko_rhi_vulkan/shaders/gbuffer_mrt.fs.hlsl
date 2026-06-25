// Render P5-r0 mesh-MRT G-buffer PRODUCER fragment shader.
//
// Writes the mesh fragment's 3 G-buffer attribute lanes in the MARCHER'S EXACT
// encoding (the contract is the marcher/resolve layout, NOT a raster-local one), with
// `mask = 1` so the deferred resolve (`deferred_pbr.hlsl`) lights mesh pixels
// first-class (full Cook-Torrance, identical to an SDF pixel). After r1's ownership
// gate the marcher YIELDS mesh-owned pixels and this fragment stands.
//
//   SV_Target0 (gAlbedo,   R8G8B8A8_UNORM): float4(saturate(base), 1) — RAW LINEAR base.
//   SV_Target1 (gNormal,   R8G8B8A8_UNORM): (oct.x, oct.y, id_ba.x, id_ba.y) — the
//     octahedral world normal in RG + the 16-bit material id in BA.
//   SV_Target2 (gMaterial, R8G8B8A8_UNORM): float4(1, 1, 1, 1) — shadow = ao = 1,
//     mask = 1 (SDF-lit selector ON), alpha 1.
//
// OQ-r0-B: `base` = interpolated LINEAR vertex color; the mesh material id is the default
// `0` (the default material). No material-table fetch / texturing (a charted follow-up).
// The vertex color MUST be linear (the gAlbedo contract is RAW LINEAR base color).
//
// DEVIATION from OQ-r0-B's "material id = a push-constant": the id is a compile-time
// constant `DEFAULT_MESH_MATERIAL_ID = 0` here, NOT a fragment push. A fragment push would
// require broadening the RHI graphics pipeline's VERTEX-only push-constant range to the
// FRAGMENT stage (a cross-cutting change to the shared graphics pipeline builder that
// every graphics pipeline — triangle/present/prepass — would inherit, beyond P5's stated
// edit surface and not validatable on this no-validation-layer box). The OUTPUT is
// byte-identical to a push of the default id 0, so r0's 0%-gate + the mesh-pixel golden
// are unaffected; wiring a real per-mesh id push lands WITH the material-table follow-up
// that actually needs a non-zero id.
//
// The `oct_encode` + `pack_material_id_ba` BODIES are SINGLE-SOURCED in
// `boyko_shaderdsl` and SPLICED here between the `// === GENERATED ... BEGIN/END ===`
// sentinels — the SAME emission the marcher consumes (one source, two splice sites).
// `gbuffer_mrt_edsl_sync.rs` guards the splice (a drift fails CI). NEVER hand-edit the
// generated bodies; re-run `cargo run -p boyko_shaderdsl --features emit --bin
// emit_field` and re-splice. The signatures + framing + raster I/O are hand-written.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer_mrt.fs.hlsl -Fo gbuffer_mrt.fs.spv

// OQ-r0-B: the mesh's 16-bit material id. The default material (id 0); see the DEVIATION
// note in the header for why this is a constant, not a fragment push, in r0.
static const uint DEFAULT_MESH_MATERIAL_ID = 0u;

struct PsIn {
    float4 position : SV_Position;
    float4 color    : COLOR0;
    float3 normal   : NORMAL;
};

struct PsOut {
    float4 albedo   : SV_Target0;  // -> gAlbedo
    float4 normal   : SV_Target1;  // -> gNormal
    float4 material : SV_Target2;  // -> gMaterial
};

// Octahedral-encode a unit normal `n` into [0,1]^2, the marcher/resolve's exact fold.
// The BODY is eDSL-single-sourced (boyko_shaderdsl::oct::oct_encode_body); the resolve
// decodes it via `oct_decode`. Spliced verbatim from the marcher's identical span.
float2 oct_encode(float3 n) {
    // === GENERATED oct_encode BEGIN === (boyko_shaderdsl::oct::oct_encode_body)
    n = n / (abs(n.x) + abs(n.y) + abs(n.z));
    float2 e = n.xy;
    if (n.z < 0.0) {
        e = (1.0 - abs(e.yx)) * float2(e.x >= 0.0 ? 1.0 : -1.0, e.y >= 0.0 ? 1.0 : -1.0);
    }
    return e * 0.5 + 0.5;
    // === GENERATED oct_encode END ===
}

// Pack a 16-bit material id into the B + A channels of an RGBA8 texel (low byte -> B,
// high byte -> A). The BODY is eDSL-single-sourced
// (boyko_shaderdsl::pack::pack_material_id_ba_body); the resolve reconstructs
// `id = round(b*255) | (round(a*255) << 8)`. Spliced verbatim from the marcher's span.
float2 pack_material_id_ba(uint id) {
    // === GENERATED pack_material_id_ba BEGIN === (boyko_shaderdsl::pack::pack_material_id_ba_body)
    uint lo = id & 255u;
    uint hi = id >> 8u & 255u;
    return float2((float)lo / 255.0, (float)hi / 255.0);
    // === GENERATED pack_material_id_ba END ===
}

PsOut main(PsIn input) {
    PsOut output;
    // gAlbedo: RAW LINEAR base color (saturated to the UNORM range), alpha 1.
    output.albedo = float4(saturate(input.color.rgb), 1.0);
    // gNormal: the octahedral world normal in RG + the packed 16-bit material id in BA.
    float3 n = normalize(input.normal);
    float2 oct = oct_encode(n);
    float2 id_ba = pack_material_id_ba(DEFAULT_MESH_MATERIAL_ID);
    output.normal = float4(oct.x, oct.y, id_ba.x, id_ba.y);
    // gMaterial: shadow = 1, ao = 1, mask = 1 (SDF-lit -> Cook-Torrance in the resolve).
    // Analytic mesh shadow/AO via the SDF march is a charted follow-up, NOT P5.
    output.material = float4(1.0, 1.0, 1.0, 1.0);
    return output;
}
